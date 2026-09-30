//! Native C++ RTTI metadata and object integration.
//!
//! [`RttiClass`] validates compiler-produced RTTI once per Rust implementation and
//! owns the prefixed callback tables. [`RttiObject::new`] then allocates objects whose
//! headers point at those shared tables; each object borrows its class. Type identity
//! and the inheritance graph come from the native compiler; ordinary
//! [`crate::OwnedObject::new`] objects do not have RTTI. The ABI inspection APIs
//! below are allocation-free. Building an [`RttiClass`] uses `alloc`.

pub use cppvtable_abi::rtti::*;

use alloc::vec::Vec;
use core::ffi::c_void;
use core::fmt;
use core::marker::PhantomData;
use core::mem::size_of;
use core::ops::Deref;

use crate::hook::ShadowVtable;
use crate::{Implement, Object, OwnedObject, VtableLayout};

/// A structural mismatch detected before installing native RTTI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RttiError {
    /// Metadata must have one entry per directly implemented interface.
    InterfaceCount,
    /// Every C++ interface needs metadata, and C interfaces must use `None`.
    InterfaceKind,
    /// The metadata uses a different C++ ABI from the interface declaration.
    AbiMismatch,
    /// The native subobject offset differs from the generated Rust object layout.
    OffsetMismatch,
    /// The interfaces describe different complete native types.
    TypeMismatch,
    /// The native locator requires construction-displacement state absent from Rust objects.
    ConstructionTable,
    /// The native hierarchy needs virtual-base object storage absent from Rust objects.
    VirtualInheritance,
    /// This RTTI representation requires a different callback table encoding.
    UnsupportedVariant,
    /// The function table cannot be represented with the native pointer layout.
    TableLayout,
}

impl fmt::Display for RttiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InterfaceCount => "RTTI metadata count does not match the interface count",
            Self::InterfaceKind => "RTTI requires metadata for C++ pointer interfaces only",
            Self::AbiMismatch => "RTTI metadata ABI does not match the interface",
            Self::OffsetMismatch => "RTTI subobject offset does not match the Rust header",
            Self::TypeMismatch => "RTTI tables describe different complete types",
            Self::ConstructionTable => "RTTI metadata requires construction-displacement state",
            Self::VirtualInheritance => "Rust RTTI objects support nonvirtual inheritance only",
            Self::UnsupportedVariant => {
                "RTTI representation cannot prefix pointer-based callback tables"
            }
            Self::TableLayout => "RTTI callback table has an unsupported size or alignment",
        })
    }
}

impl core::error::Error for RttiError {}

/// Validated native RTTI for every object of the Rust implementation `T`.
///
/// Build one per implementation and native class, then create objects with
/// [`RttiObject::new`]. Objects copy the class's interface headers, so creating one
/// costs no more than [`crate::OwnedObject::new`]. The class owns the prefixed
/// callback tables; every object borrows the class, so it cannot be dropped first.
pub struct RttiClass<T: Implement> {
    vtables: T::Vtables,
    // Owns the tables addressed by `vtables`.
    _tables: Vec<ShadowVtable>,
}

impl<T: Implement> RttiClass<T> {
    /// Validate native metadata and build the prefixed callback tables.
    ///
    /// Supply metadata in `#[implement]` interface order. Every C++ interface needs
    /// `Some(metadata)`; C interfaces use `None`. The metadata can be captured from
    /// compiler-generated tables using [`RttiMetadata`].
    ///
    /// # Errors
    ///
    /// Returns [`RttiError`] for mismatched counts, interface kinds, ABIs, type
    /// identities, offsets, unsupported table layouts or RTTI representations,
    /// construction tables, or Microsoft metadata marked with virtual inheritance.
    /// Checked structural mismatches may be supplied and are rejected here.
    ///
    /// # Safety
    ///
    /// Metadata that passes the structural checks must describe one compatible
    /// complete C++ class, with exactly the same interface subobjects, inheritance,
    /// offsets, and callback contracts. Only nonvirtual inheritance is supported for
    /// Rust-created objects; this is checked for Microsoft metadata only. All base
    /// subobjects reachable by RTTI must exist at the declared offsets; equal offsets
    /// and type names alone do not establish this. Native code may call declared
    /// virtual methods and use RTTI on objects built from this class, but must not
    /// access undeclared C++ data, invoke constructors/destructors, or delete a Rust
    /// allocation. Callback calls must dispatch through the table: native
    /// final/devirtualized method bodies must not replace Rust callbacks. The metadata
    /// contract of [`RttiMetadata`] applies for as long as objects of this class exist.
    pub unsafe fn new(metadata: &[Option<RttiMetadata>]) -> Result<Self, RttiError> {
        if metadata.len() != T::INTERFACES.len() {
            return Err(RttiError::InterfaceCount);
        }
        let mut vtables = T::vtables();
        let headers = core::ptr::from_mut(&mut vtables).cast::<u8>();
        let mut tables = Vec::new();
        let mut type_info = None;
        for (index, (descriptor, metadata)) in T::INTERFACES.iter().zip(metadata).enumerate() {
            let Some(abi) = descriptor.cpp_abi else {
                if metadata.is_some() {
                    return Err(RttiError::InterfaceKind);
                }
                continue;
            };
            let metadata = metadata.ok_or(RttiError::InterfaceKind)?;
            if descriptor.layout != VtableLayout::Pointer {
                return Err(RttiError::InterfaceKind);
            }
            if metadata.abi() != abi {
                return Err(RttiError::AbiMismatch);
            }
            if !metadata.supports_pointer_tables() {
                return Err(RttiError::UnsupportedVariant);
            }
            let offset = T::SLOT_OFFSETS[index];
            let signed_offset = isize::try_from(offset).map_err(|_| RttiError::OffsetMismatch)?;
            if metadata.offset_to_top() != -signed_offset {
                return Err(RttiError::OffsetMismatch);
            }
            if metadata.construction_displacement() != 0 {
                return Err(RttiError::ConstructionTable);
            }
            if metadata
                .msvc_hierarchy_flags()
                .is_some_and(|flags| flags & 2 != 0)
            {
                return Err(RttiError::VirtualInheritance);
            }
            let identity = metadata.type_info();
            if type_info.is_some_and(|previous| previous != identity) {
                return Err(RttiError::TypeMismatch);
            }
            type_info = Some(identity);
            let entry = size_of::<*const c_void>();
            if descriptor.table_size % entry != 0 || descriptor.table_align > entry {
                return Err(RttiError::TableLayout);
            }
            let table =
                ShadowVtable::allocate(metadata.prefix_size(), descriptor.table_size / entry)
                    .ok_or(RttiError::TableLayout)?;
            // SAFETY: The allocation holds the prefix followed by `table_size` bytes.
            // Implement's contract supplies a static table of exactly that size, and
            // places a pointer-layout header at `offset` inside `Vtables`.
            unsafe {
                metadata.write_prefix(table.prefix());
                core::ptr::copy_nonoverlapping(
                    T::vtable_slots()[index].as_ptr().cast::<u8>(),
                    table.address_point().cast_mut().cast::<u8>(),
                    descriptor.table_size,
                );
                headers
                    .add(offset)
                    .cast::<*const c_void>()
                    .write_unaligned(table.address_point());
            }
            tables.push(table);
        }
        Ok(Self {
            vtables,
            _tables: tables,
        })
    }

    /// Interface headers for a new object; C++ entries address the owned tables.
    pub(crate) fn vtables(&self) -> T::Vtables {
        self.vtables
    }
}

/// Sole ownership of an object whose C++ interfaces carry native RTTI.
///
/// Dereferences to [`OwnedObject`] for values and interfaces. The borrow of its
/// [`RttiClass`] keeps the prefixed callback tables alive until the object is gone:
///
/// ```compile_fail,E0515
/// use cppvtable::Implement;
/// use cppvtable::rtti::{RttiClass, RttiObject};
/// fn outlive<T: Implement>(value: T, class: RttiClass<T>) -> RttiObject<'static, T> {
///     RttiObject::new(value, &class)
/// }
/// ```
pub struct RttiObject<'c, T: Implement> {
    owner: OwnedObject<T>,
    class: PhantomData<&'c RttiClass<T>>,
}

impl<'c, T: Implement> RttiObject<'c, T> {
    /// Allocate an object using the headers and tables of `class`.
    #[must_use]
    pub fn new(value: T, class: &'c RttiClass<T>) -> Self {
        Self {
            owner: OwnedObject::with_headers(class.vtables(), value),
            class: PhantomData,
        }
    }

    /// Transfer allocation ownership to a raw pointer.
    ///
    /// Reclaim it with [`Self::from_raw`] or [`OwnedObject::from_raw`] while the class
    /// is still alive.
    #[must_use]
    pub fn into_raw(self) -> *mut Object<T> {
        self.owner.into_raw()
    }

    /// Reclaim ownership of an object created from `class`.
    ///
    /// # Safety
    ///
    /// `object` must satisfy [`OwnedObject::from_raw`] and have been created by
    /// [`Self::new`] with `class`.
    #[must_use]
    pub unsafe fn from_raw(object: *mut Object<T>, class: &'c RttiClass<T>) -> Self {
        let _ = class;
        Self {
            // SAFETY: The caller transfers sole ownership of a live allocation.
            owner: unsafe { OwnedObject::from_raw(object) },
            class: PhantomData,
        }
    }
}

impl<T: Implement> Deref for RttiObject<'_, T> {
    type Target = OwnedObject<T>;

    fn deref(&self) -> &OwnedObject<T> {
        &self.owner
    }
}
