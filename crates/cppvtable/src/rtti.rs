//! Native C++ RTTI metadata and object integration.
//!
//! [`crate::OwnedObject::new_with_rtti`] installs compiler-produced RTTI alongside
//! Rust callback tables. Type identity and the inheritance graph come from the
//! native compiler; ordinary [`crate::OwnedObject::new`] objects do not have RTTI.
//! The ABI inspection APIs below are allocation-free. Owned RTTI tables use `alloc`.

pub use cppvtable_abi::rtti::*;

use alloc::alloc::{alloc, dealloc, handle_alloc_error};
use alloc::vec::Vec;
use core::alloc::Layout;
use core::fmt;
use core::ptr::NonNull;

use crate::{Implement, VtableLayout};

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

/// Allocation holding the RTTI prefix followed by a copy of a Rust callback table.
struct Table {
    allocation: NonNull<u8>,
    layout: Layout,
    prefix_size: usize,
}

impl Table {
    fn new(
        metadata: RttiMetadata,
        source: *const core::ffi::c_void,
        size: usize,
        alignment: usize,
    ) -> Result<Self, RttiError> {
        let prefix_size = metadata.prefix_size();
        let pointer_size = core::mem::size_of::<usize>();
        if size == 0 || size % pointer_size != 0 || alignment > pointer_size {
            return Err(RttiError::TableLayout);
        }
        let layout = Layout::from_size_align(
            prefix_size
                .checked_add(size)
                .ok_or(RttiError::TableLayout)?,
            pointer_size,
        )
        .map_err(|_| RttiError::TableLayout)?;
        // SAFETY: The checked layout is nonzero and valid.
        let raw = unsafe { alloc(layout) };
        let allocation = NonNull::new(raw).unwrap_or_else(|| handle_alloc_error(layout));
        let table = Self {
            allocation,
            layout,
            prefix_size,
        };
        // SAFETY: Implement's contract supplies a static table of this exact size.
        // The new allocation has room for the aligned prefix and the entire table.
        unsafe {
            metadata.write_prefix(raw);
            core::ptr::copy_nonoverlapping(source.cast::<u8>(), raw.add(prefix_size), size);
        }
        Ok(table)
    }

    fn address(&self) -> *const core::ffi::c_void {
        // SAFETY: The prefix is followed by a nonempty table in this allocation.
        unsafe { self.allocation.as_ptr().add(self.prefix_size).cast() }
    }
}

impl Drop for Table {
    fn drop(&mut self) {
        // SAFETY: This object uniquely owns the allocation with its original layout.
        unsafe { dealloc(self.allocation.as_ptr(), self.layout) };
    }
}

/// Opaque auxiliary storage owned by an RTTI object's allocation.
///
/// Constructed by [`crate::OwnedObject::new_with_rtti`]. Keeping this type in the raw
/// allocation pointer ensures `from_raw` restores the same layout and table ownership.
pub struct RttiTables(Vec<(usize, Table)>);

// SAFETY: Tables are immutable after installation and contain only callback pointers
// and permanently loaded native metadata. Their unique owner controls deallocation.
unsafe impl Send for RttiTables {}
// SAFETY: Shared accesses only read immutable table storage.
unsafe impl Sync for RttiTables {}

impl RttiTables {
    pub(crate) fn new<T: Implement>(metadata: &[Option<RttiMetadata>]) -> Result<Self, RttiError> {
        if metadata.len() != T::INTERFACES.len() {
            return Err(RttiError::InterfaceCount);
        }
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
            let table = Table::new(
                metadata,
                T::vtable_slots()[index].as_ptr(),
                descriptor.table_size,
                descriptor.table_align,
            )?;
            tables.push((offset, table));
        }
        Ok(Self(tables))
    }

    /// Install only the pointer-layout headers checked by `new`.
    pub(crate) unsafe fn install(&self, headers: *mut u8) {
        for (offset, table) in &self.0 {
            // SAFETY: The caller supplies the matching live writable header storage.
            unsafe {
                headers
                    .add(*offset)
                    .cast::<*const core::ffi::c_void>()
                    .write_unaligned(table.address());
            };
        }
    }
}
