//! Inspection of compiler-produced C++ RTTI and optional native cast adapters.
//!
//! This module borrows native type descriptors and hierarchy information. It does not
//! invent C++ type identities or require linking a C++ runtime. Runtime cast functions
//! can be supplied explicitly when casts beyond complete-object recovery are needed.
//! Extracted layout, hierarchy, and raw-name fields must remain immutable and loaded
//! for the rest of the process. Independent native demangling caches are never read.
//!
//! The pointer-based Itanium layout follows the
//! [Itanium C++ ABI](https://itanium-cxx-abi.github.io/cxx-abi/abi.html#rtti).
//! Microsoft descriptor layouts follow
//! [Clang's Microsoft ABI implementation](https://github.com/llvm/llvm-project/blob/main/clang/lib/CodeGen/MicrosoftCXXABI.cpp);
//! its cast signature is documented by
//! [Microsoft](https://learn.microsoft.com/en-us/cpp/c-runtime-library/rtdynamiccast).
//! Clang relative vtables can be inspected explicitly, but cannot be installed using
//! pointer-table callback storage. Construction/destruction-time objects are excluded.
//! Apple arm64's tagged type-name field follows
//! [libc++](https://github.com/llvm/llvm-project/blob/main/libcxx/include/typeinfo).
//! Authenticated vtable/type-info/function pointers, including arm64e, are excluded.

use core::ffi::{CStr, c_char, c_void};
use core::mem::size_of;

/// The C++ object ABI used by native RTTI metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CppAbi {
    /// Microsoft C++ ABI, including clang-cl.
    Msvc,
    /// Itanium C++ ABI family, including Clang's relative-table representation.
    Itanium,
}

/// Physical RTTI representation; this cannot always be inferred from the ABI family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RttiVariant {
    /// Microsoft revision-zero locator containing native absolute pointers.
    MsvcAbsolute,
    /// Microsoft revision-one locator containing 32-bit image-relative addresses.
    MsvcImageRelative,
    /// Itanium pointer-sized vtable components and a direct type descriptor pointer.
    ItaniumPointer,
    /// Unsigned Apple arm64 pointer components with a high-bit-tagged type-name field.
    ItaniumAppleArm64,
    /// Clang 32-bit relative components and an indirect descriptor proxy, with
    /// ordinary untagged Itanium type-name pointers.
    ItaniumRelative32,
}

impl RttiVariant {
    /// Calling-convention family associated with this RTTI representation.
    #[must_use]
    pub const fn abi(self) -> CppAbi {
        match self {
            Self::MsvcAbsolute | Self::MsvcImageRelative => CppAbi::Msvc,
            Self::ItaniumPointer | Self::ItaniumAppleArm64 | Self::ItaniumRelative32 => {
                CppAbi::Itanium
            }
        }
    }
}

/// Two words immediately preceding an Itanium vtable's function address point.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ItaniumPrefix {
    /// Signed byte displacement from this interface to the complete object.
    pub offset_to_top: isize,
    /// Native `std::type_info` object for the complete dynamic type.
    pub type_info: *const c_void,
}

/// Two 32-bit words preceding a Clang relative vtable's function address point.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ItaniumRelativePrefix {
    /// Signed byte displacement from this interface to the complete object.
    pub offset_to_top: i32,
    /// Signed displacement from the function address point to a type-info proxy.
    pub type_info_proxy: i32,
}

/// Pointer immediately preceding a Microsoft vftable's function address point.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MsvcPrefix {
    /// Compiler-produced complete object locator, using the target's MSVC layout.
    pub locator: *const c_void,
}

/// Microsoft revision-zero complete object locator with target-sized pointers.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MsvcAbsoluteLocator {
    /// Revision zero denotes the absolute-pointer representation.
    pub signature: u32,
    /// Byte displacement from the complete object to this interface's vfptr.
    pub offset: u32,
    /// Offset of a signed construction displacement before this interface, or zero.
    pub construction_displacement: u32,
    /// Native type descriptor address.
    pub type_descriptor: *const c_void,
    /// Native class hierarchy descriptor address.
    pub class_descriptor: *const c_void,
}

/// Microsoft revision-one complete object locator with image-relative addresses.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MsvcRelativeLocator {
    /// Revision one denotes the image-relative representation.
    pub signature: u32,
    /// Byte displacement from the complete object to this interface's vfptr.
    pub offset: u32,
    /// Offset of a signed construction displacement before this interface, or zero.
    pub construction_displacement: u32,
    /// Image-relative byte address of the native type descriptor.
    pub type_descriptor: u32,
    /// Image-relative byte address of the class hierarchy descriptor.
    pub class_descriptor: u32,
    /// Image-relative byte address of this locator itself.
    pub self_offset: u32,
}

#[derive(Clone, Copy, Debug)]
enum Prefix {
    Itanium(ItaniumPrefix),
    ItaniumRelative {
        offset_to_top: isize,
        type_info: *const c_void,
    },
    Msvc {
        locator: *const c_void,
        type_info: *const c_void,
        hierarchy: *const c_void,
        offset: isize,
        construction_displacement: u32,
    },
}

/// A copyable view of immutable native RTTI metadata.
///
/// This view outlives the object used to extract it, because its unsafe constructors
/// require the native descriptors and their names to remain loaded permanently. It
/// does not own the source object or any callback table. Reusing the metadata for a
/// Rust object additionally requires the native hierarchy and Rust object layout to
/// agree; extraction alone does not establish that contract.
/// Independent native demangling-cache fields may change; this view never reads them.
#[derive(Clone, Copy, Debug)]
pub struct RttiMetadata {
    prefix: Prefix,
    variant: RttiVariant,
}

// SAFETY: Constructors require all referenced metadata and names to be immutable and
// permanently loaded. Reading or copying this metadata does not access object state.
unsafe impl Send for RttiMetadata {}
// SAFETY: See the immutable-metadata contract of Send.
unsafe impl Sync for RttiMetadata {}

impl RttiMetadata {
    /// Extract RTTI from a native interface using its family's ordinary defaults.
    ///
    /// Itanium selects pointer-sized components, with Apple arm64 tagged names on
    /// that target. Microsoft chooses the locator's
    /// revision-zero absolute or revision-one image-relative representation. Use
    /// [`Self::from_interface_variant`] for an explicitly selected representation.
    ///
    /// # Safety
    ///
    /// `object` must be a live, fully constructed polymorphic interface of `abi`, with
    /// RTTI enabled and the ordinary pointer-based vtable representation. It must not
    /// be in construction or destruction. Its RTTI descriptors and terminated names
    /// must be immutable, valid, and loaded for the rest of the process. This includes
    /// every image-relative Microsoft descriptor referenced by the locator.
    /// All object, descriptor, and function pointers must be unsigned; authenticated
    /// representations such as arm64e require a native adapter instead.
    ///
    /// # Panics
    ///
    /// Panics for a null pointer or unsupported Microsoft locator revision.
    #[must_use]
    pub unsafe fn from_interface(abi: CppAbi, object: *mut c_void) -> Self {
        assert!(
            !object.is_null(),
            "RTTI requires a non-null polymorphic interface"
        );
        let table = unsafe { object.cast::<*const c_void>().read() };
        unsafe { Self::from_vtable(abi, table) }
    }

    /// Extract RTTI from an interface using an explicit physical representation.
    ///
    /// # Safety
    ///
    /// The live fully constructed interface must use `variant`, and its metadata
    /// must satisfy the permanent lifetime contract of [`Self::from_interface`].
    /// The interface's first field must be its vtable address-point pointer.
    ///
    /// # Panics
    ///
    /// Panics for null pointers or a mismatched Microsoft locator revision.
    #[must_use]
    pub unsafe fn from_interface_variant(variant: RttiVariant, object: *mut c_void) -> Self {
        assert!(
            !object.is_null(),
            "RTTI requires a non-null polymorphic interface"
        );
        let table = unsafe { object.cast::<*const c_void>().read() };
        unsafe { Self::from_vtable_variant(variant, table) }
    }

    /// Extract RTTI from a vtable address point using the family's ordinary defaults.
    ///
    /// # Safety
    ///
    /// `table` must address a valid complete-object vtable with the RTTI prefix for
    /// `abi`; it must not be a constructor/destructor table or a relative Itanium
    /// table. The descriptors and terminated names must obey the immutable, permanent
    /// lifetime contract of [`Self::from_interface`].
    /// Its pointers must be unsigned rather than requiring authentication.
    ///
    /// # Panics
    ///
    /// Panics for null pointers or an unsupported Microsoft locator revision.
    #[must_use]
    pub unsafe fn from_vtable(abi: CppAbi, table: *const c_void) -> Self {
        assert!(
            !table.is_null(),
            "RTTI requires a non-null vtable address point"
        );
        match abi {
            CppAbi::Itanium => unsafe {
                Self::from_vtable_variant(default_itanium_variant(), table)
            },
            CppAbi::Msvc => {
                let prefix = unsafe { table.cast::<MsvcPrefix>().sub(1).read() };
                unsafe { Self::from_msvc_prefix(prefix) }
            }
        }
    }

    /// Extract RTTI from the first function slot using an explicit representation.
    ///
    /// # Safety
    ///
    /// `table` must be a complete-object address point using `variant`. Its prefix
    /// and any relative proxy must be readable and immutable during extraction.
    /// Referenced descriptors and raw-name fields must remain valid, immutable, and
    /// loaded permanently. For Microsoft metadata this includes the locator itself,
    /// whose pointer is retained. Relative32 requires ordinary untagged type names;
    /// combining relative tables with Apple arm64 tagged names is not supported.
    /// Construction and destruction tables are excluded.
    /// Object and descriptor pointers must be unsigned, including on Apple targets.
    ///
    /// # Panics
    ///
    /// Panics for null pointers or a mismatched Microsoft locator revision.
    #[must_use]
    pub unsafe fn from_vtable_variant(variant: RttiVariant, table: *const c_void) -> Self {
        assert!(
            !table.is_null(),
            "RTTI requires a non-null vtable address point"
        );
        match variant {
            RttiVariant::ItaniumPointer | RttiVariant::ItaniumAppleArm64 => {
                let prefix = unsafe { table.cast::<ItaniumPrefix>().sub(1).read() };
                unsafe { Self::from_itanium_prefix_variant(variant, prefix) }
            }
            RttiVariant::ItaniumRelative32 => {
                let prefix = unsafe { table.cast::<ItaniumRelativePrefix>().sub(1).read() };
                unsafe { Self::from_itanium_relative_prefix(prefix, table) }
            }
            RttiVariant::MsvcAbsolute | RttiVariant::MsvcImageRelative => {
                let prefix = unsafe { table.cast::<MsvcPrefix>().sub(1).read() };
                unsafe { Self::from_msvc_prefix_variant(variant, prefix) }
            }
        }
    }

    /// Describe an Itanium RTTI prefix supplied by the caller.
    ///
    /// # Safety
    ///
    /// `prefix.type_info` must address valid native class RTTI, with all descriptors
    /// and terminated names immutable and loaded for the rest of the process.
    /// The offset must correspond to a complete-object interface address point.
    ///
    /// # Panics
    ///
    /// Panics if the type descriptor is null.
    #[must_use]
    pub unsafe fn from_itanium_prefix(prefix: ItaniumPrefix) -> Self {
        unsafe { Self::from_itanium_prefix_variant(default_itanium_variant(), prefix) }
    }

    /// Describe pointer-sized Itanium metadata with explicit name encoding.
    ///
    /// # Safety
    ///
    /// The prefix must satisfy [`Self::from_itanium_prefix`]'s contract and use the
    /// selected ordinary or Apple arm64 tagged-name encoding. Object and descriptor
    /// pointers must not require authentication.
    ///
    /// # Panics
    ///
    /// Panics for a null descriptor or a variant without pointer-sized Itanium entries.
    #[must_use]
    pub unsafe fn from_itanium_prefix_variant(variant: RttiVariant, prefix: ItaniumPrefix) -> Self {
        assert!(
            matches!(
                variant,
                RttiVariant::ItaniumPointer | RttiVariant::ItaniumAppleArm64
            ),
            "expected pointer-sized Itanium RTTI"
        );
        assert!(
            !prefix.type_info.is_null(),
            "RTTI requires a native type descriptor"
        );
        Self {
            prefix: Prefix::Itanium(prefix),
            variant,
        }
    }

    /// Resolve a Clang relative prefix and its indirect type-info proxy.
    ///
    /// # Safety
    ///
    /// `address_point` and `prefix` must describe the same valid complete-object
    /// relative table. The pointer-sized proxy must be readable and immutable during
    /// extraction. Referenced native RTTI and raw-name fields must remain immutable
    /// and loaded permanently. Type-name pointers must use ordinary untagged Itanium
    /// encoding; combining relative tables with Apple arm64 tagged names is unsupported.
    ///
    /// # Panics
    ///
    /// Panics if the address point or resolved type descriptor is null.
    #[must_use]
    pub unsafe fn from_itanium_relative_prefix(
        prefix: ItaniumRelativePrefix,
        address_point: *const c_void,
    ) -> Self {
        assert!(
            !address_point.is_null(),
            "RTTI requires a vtable address point"
        );
        let proxy = address_point
            .cast::<u8>()
            .wrapping_offset(prefix.type_info_proxy as isize);
        let type_info = unsafe { proxy.cast::<*const c_void>().read_unaligned() };
        assert!(
            !type_info.is_null(),
            "RTTI requires a native type descriptor"
        );
        Self {
            prefix: Prefix::ItaniumRelative {
                offset_to_top: prefix.offset_to_top as isize,
                type_info,
            },
            variant: RttiVariant::ItaniumRelative32,
        }
    }

    /// Describe a Microsoft RTTI prefix supplied by the caller.
    ///
    /// # Safety
    ///
    /// The locator, descriptors, and terminated names must be valid for the target's
    /// Microsoft ABI, immutable, and loaded for the rest of the process. Revision zero
    /// selects native absolute pointers; revision one selects image-relative fields.
    /// The locator must describe a complete-object interface, with representable
    /// object offsets.
    ///
    /// # Panics
    ///
    /// Panics for null pointers, unrepresentable offsets, or an unsupported revision.
    #[must_use]
    pub unsafe fn from_msvc_prefix(prefix: MsvcPrefix) -> Self {
        assert!(
            !prefix.locator.is_null(),
            "RTTI requires a complete object locator"
        );
        let revision = unsafe { prefix.locator.cast::<u32>().read() };
        let variant = match revision {
            0 => RttiVariant::MsvcAbsolute,
            1 => RttiVariant::MsvcImageRelative,
            _ => panic!("unsupported Microsoft RTTI locator revision"),
        };
        unsafe { Self::from_msvc_prefix_variant(variant, prefix) }
    }

    /// Decode a Microsoft locator using an explicitly selected representation.
    ///
    /// # Safety
    ///
    /// The locator and native pointer fields must have the selected representation
    /// on this target and satisfy [`Self::from_msvc_prefix`]'s metadata contract.
    ///
    /// # Panics
    ///
    /// Panics for non-Microsoft variants, null pointers, unrepresentable offsets,
    /// or a locator revision that does not match `variant`.
    #[must_use]
    pub unsafe fn from_msvc_prefix_variant(variant: RttiVariant, prefix: MsvcPrefix) -> Self {
        assert!(
            !prefix.locator.is_null(),
            "RTTI requires a complete object locator"
        );
        let (type_info, hierarchy, offset, displacement) = match variant {
            RttiVariant::MsvcImageRelative => {
                assert_eq!(
                    unsafe { prefix.locator.cast::<u32>().read() },
                    1,
                    "image-relative Microsoft RTTI requires a revision-one locator"
                );
                let locator = unsafe { prefix.locator.cast::<MsvcRelativeLocator>().read() };
                let image = prefix
                    .locator
                    .cast::<u8>()
                    .wrapping_sub(locator.self_offset as usize);
                (
                    image
                        .wrapping_add(locator.type_descriptor as usize)
                        .cast::<c_void>(),
                    image
                        .wrapping_add(locator.class_descriptor as usize)
                        .cast::<c_void>(),
                    locator.offset,
                    locator.construction_displacement,
                )
            }
            RttiVariant::MsvcAbsolute => {
                assert_eq!(
                    unsafe { prefix.locator.cast::<u32>().read() },
                    0,
                    "absolute Microsoft RTTI requires a revision-zero locator"
                );
                let locator = unsafe { prefix.locator.cast::<MsvcAbsoluteLocator>().read() };
                (
                    locator.type_descriptor,
                    locator.class_descriptor,
                    locator.offset,
                    locator.construction_displacement,
                )
            }
            _ => panic!("expected a Microsoft RTTI representation"),
        };
        assert!(
            !type_info.is_null(),
            "RTTI requires a native type descriptor"
        );
        Self {
            prefix: Prefix::Msvc {
                locator: prefix.locator,
                type_info,
                hierarchy,
                offset: isize::try_from(offset).expect("native object offsets must fit ptrdiff_t"),
                construction_displacement: displacement,
            },
            variant,
        }
    }

    /// The object ABI represented by this metadata.
    #[must_use]
    pub const fn abi(self) -> CppAbi {
        self.variant.abi()
    }

    /// Explicit physical representation of this metadata.
    #[must_use]
    pub const fn variant(self) -> RttiVariant {
        self.variant
    }

    /// Whether this metadata can prefix ordinary pointer-sized callback tables.
    #[must_use]
    pub const fn supports_pointer_tables(self) -> bool {
        !matches!(self.variant, RttiVariant::ItaniumRelative32)
    }

    /// Native type descriptor address, suitable for matching native `typeid` results.
    #[must_use]
    pub const fn type_info(self) -> *const c_void {
        match self.prefix {
            Prefix::Itanium(prefix) => prefix.type_info,
            Prefix::ItaniumRelative { type_info, .. } | Prefix::Msvc { type_info, .. } => type_info,
        }
    }

    /// Signed displacement to the complete object, excluding construction adjustment.
    #[must_use]
    pub const fn offset_to_top(self) -> isize {
        match self.prefix {
            Prefix::Itanium(prefix) => prefix.offset_to_top,
            Prefix::ItaniumRelative { offset_to_top, .. } => offset_to_top,
            Prefix::Msvc { offset, .. } => -offset,
        }
    }

    /// Offset of the Microsoft construction-displacement field; zero for Itanium.
    ///
    /// Nonzero values require additional object state and cannot be cloned into the
    /// ordinary Rust interface-header layout solely by copying a vtable prefix.
    #[must_use]
    pub const fn construction_displacement(self) -> u32 {
        match self.prefix {
            Prefix::Itanium(_) | Prefix::ItaniumRelative { .. } => 0,
            Prefix::Msvc {
                construction_displacement,
                ..
            } => construction_displacement,
        }
    }

    /// Microsoft class-hierarchy flags, or `None` for an Itanium descriptor.
    ///
    /// Microsoft bit 1 (`0x2`) denotes virtual inheritance. Its absence does not prove
    /// that a native hierarchy matches a Rust object's complete layout.
    #[must_use]
    pub fn msvc_hierarchy_flags(self) -> Option<u32> {
        match self.prefix {
            Prefix::Itanium(_) | Prefix::ItaniumRelative { .. } => None,
            Prefix::Msvc { hierarchy, .. } => {
                if hierarchy.is_null() {
                    return None;
                }
                Some(unsafe { hierarchy.cast::<u32>().add(1).read() })
            }
        }
    }

    /// Raw compiler-produced name, without demangling or invoking native code.
    #[must_use]
    pub fn mangled_name(self) -> &'static CStr {
        let descriptor = self.type_info();
        let name = match self.abi() {
            CppAbi::Itanium => unsafe { descriptor.cast::<*const c_char>().add(1).read() },
            CppAbi::Msvc => unsafe {
                descriptor
                    .cast::<u8>()
                    .add(2 * size_of::<*const c_void>())
                    .cast::<c_char>()
            },
        };
        let name = decode_name_pointer(self.variant, name);
        unsafe { CStr::from_ptr(name) }
    }

    /// Number of bytes required immediately before the vtable function address point.
    #[must_use]
    pub const fn prefix_size(self) -> usize {
        match self.prefix {
            Prefix::Itanium(_) => size_of::<ItaniumPrefix>(),
            Prefix::ItaniumRelative { .. } => size_of::<ItaniumRelativePrefix>(),
            Prefix::Msvc { .. } => size_of::<MsvcPrefix>(),
        }
    }

    /// Write the native prefix into memory before a cloned callback table.
    ///
    /// This writes only the prefix and retains native type/hierarchy identity. It does
    /// not copy callbacks, virtual-base offsets, or any object fields.
    ///
    /// # Safety
    ///
    /// `destination` must address writable storage of at least [`Self::prefix_size`]
    /// bytes. No reference or other access may overlap the write.
    ///
    /// # Panics
    ///
    /// Panics for relative Itanium metadata. Its proxy displacement cannot be copied
    /// into arbitrary storage, and ordinary callback tables use incompatible entries.
    pub unsafe fn write_prefix(self, destination: *mut u8) {
        match self.prefix {
            Prefix::Itanium(prefix) => unsafe {
                destination.cast::<ItaniumPrefix>().write_unaligned(prefix);
            },
            Prefix::ItaniumRelative { .. } => {
                panic!("relative Itanium RTTI requires relocated relative callback storage")
            }
            Prefix::Msvc { locator, .. } => unsafe {
                destination
                    .cast::<MsvcPrefix>()
                    .write_unaligned(MsvcPrefix { locator });
            },
        }
    }

    /// Recover the complete native object from an interface address.
    ///
    /// # Safety
    ///
    /// The non-null `object` must be a live interface described by this metadata. All
    /// offset calculations must remain in that complete object allocation, including
    /// any Microsoft signed construction-displacement field. Null is returned unchanged.
    #[must_use]
    pub unsafe fn complete_object(self, object: *mut c_void) -> *mut c_void {
        if object.is_null() {
            return object;
        }
        let mut top = object.cast::<u8>().wrapping_offset(self.offset_to_top());
        if let Prefix::Msvc {
            construction_displacement,
            ..
        } = self.prefix
        {
            if construction_displacement != 0 {
                let displacement = unsafe {
                    object
                        .cast::<u8>()
                        .sub(construction_displacement as usize)
                        .cast::<i32>()
                        .read_unaligned()
                };
                top = top.wrapping_offset(-(displacement as isize));
            }
        }
        top.cast()
    }
}

const fn default_itanium_variant() -> RttiVariant {
    if cfg!(all(target_vendor = "apple", target_arch = "aarch64")) {
        RttiVariant::ItaniumAppleArm64
    } else {
        RttiVariant::ItaniumPointer
    }
}

fn decode_name_pointer(variant: RttiVariant, name: *const c_char) -> *const c_char {
    if variant == RttiVariant::ItaniumAppleArm64 {
        name.map_addr(|address| address & (usize::MAX >> 1))
    } else {
        name
    }
}

/// Resolve a function entry in a Clang relative vtable.
///
/// Relative callback and RTTI-proxy displacements are measured from the function
/// address point. They are not measured from each individual slot. This helper does
/// not authenticate pointers or adapt the resolved function's calling convention.
///
/// # Safety
///
/// `address_point` must identify a live relative vtable with a function entry at
/// `slot`. That entry must resolve to a valid function using this Clang representation.
/// The caller must use the exact native signature when invoking the returned address.
#[must_use]
pub unsafe fn relative_function(address_point: *const i32, slot: usize) -> *const c_void {
    let displacement = unsafe { address_point.add(slot).read() };
    address_point
        .cast::<u8>()
        .wrapping_offset(displacement as isize)
        .cast()
}

/// ABI signature of the Itanium C++ runtime's pointer `__dynamic_cast` operation.
pub type ItaniumDynamicCast = unsafe extern "C" fn(
    object: *const c_void,
    source_type: *const c_void,
    target_type: *const c_void,
    source_to_target_hint: isize,
) -> *mut c_void;

/// ABI signature of Microsoft's `__RTDynamicCast` operation.
pub type MsvcDynamicCast = unsafe extern "C" fn(
    object: *mut c_void,
    vfptr_delta: i32,
    source_type: *const c_void,
    target_type: *const c_void,
    is_reference: i32,
) -> *mut c_void;

/// Explicit native runtime adapter; constructing it does not link any runtime symbol.
#[derive(Clone, Copy)]
pub enum DynamicCastRuntime {
    /// Native Itanium `__dynamic_cast`, or a compatible nonthrowing wrapper.
    Itanium(ItaniumDynamicCast),
    /// Native Microsoft `__RTDynamicCast`, or a compatible nonthrowing wrapper.
    Msvc(MsvcDynamicCast),
}

impl DynamicCastRuntime {
    /// The ABI of the supplied cast function.
    #[must_use]
    pub const fn abi(self) -> CppAbi {
        match self {
            Self::Itanium(_) => CppAbi::Itanium,
            Self::Msvc(_) => CppAbi::Msvc,
        }
    }

    /// Perform a native downcast or cross-cast with the general pointer-cast defaults.
    ///
    /// Failed and null-source casts return null. This does not implement arbitrary
    /// static upcasts; supply a C++ wrapper for casts resolved without native RTTI.
    ///
    /// # Safety
    ///
    /// The function must implement the selected ABI and remain loaded throughout the
    /// call. `object` must be a live polymorphic source interface with its vfptr at
    /// offset zero, or null. The source and target descriptors must denote valid class
    /// RTTI from the same ABI/runtime as the object, with source matching its static
    /// interface type. The supplied runtime must understand the object's physical
    /// vtable and RTTI variant; an ordinary Itanium runtime need not accept Clang
    /// relative tables. All native object/hierarchy access must be valid. No exception
    /// may cross the Rust FFI boundary.
    #[must_use]
    pub unsafe fn cast(
        self,
        object: *mut c_void,
        source_type: *const c_void,
        target_type: *const c_void,
    ) -> *mut c_void {
        if object.is_null() {
            return object;
        }
        match self {
            Self::Itanium(function) => unsafe {
                function(object.cast_const(), source_type, target_type, -1)
            },
            Self::Msvc(function) => unsafe { function(object, 0, source_type, target_type, 0) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ItaniumPrefix, ItaniumRelativePrefix, MsvcAbsoluteLocator, MsvcPrefix, MsvcRelativeLocator,
        RttiVariant, decode_name_pointer,
    };
    use core::ffi::CStr;
    use core::mem::{offset_of, size_of};

    #[test]
    fn rtti_prefixes_use_the_native_address_point_layout() {
        let pointer = size_of::<*const core::ffi::c_void>();
        assert_eq!(offset_of!(ItaniumPrefix, offset_to_top), 0);
        assert_eq!(offset_of!(ItaniumPrefix, type_info), pointer);
        assert_eq!(size_of::<ItaniumPrefix>(), 2 * pointer);
        assert_eq!(size_of::<ItaniumRelativePrefix>(), 8);
        assert_eq!(offset_of!(ItaniumRelativePrefix, offset_to_top), 0);
        assert_eq!(offset_of!(ItaniumRelativePrefix, type_info_proxy), 4);
        assert_eq!(size_of::<MsvcPrefix>(), pointer);
        assert_eq!(offset_of!(MsvcPrefix, locator), 0);
    }

    #[test]
    fn microsoft_locators_preserve_the_documented_wire_fields() {
        assert_eq!(size_of::<MsvcRelativeLocator>(), 24);
        assert_eq!(offset_of!(MsvcRelativeLocator, signature), 0);
        assert_eq!(offset_of!(MsvcRelativeLocator, offset), 4);
        assert_eq!(
            offset_of!(MsvcRelativeLocator, construction_displacement),
            8
        );
        assert_eq!(offset_of!(MsvcRelativeLocator, type_descriptor), 12);
        assert_eq!(offset_of!(MsvcRelativeLocator, class_descriptor), 16);
        assert_eq!(offset_of!(MsvcRelativeLocator, self_offset), 20);
        assert_eq!(offset_of!(MsvcAbsoluteLocator, signature), 0);
        assert_eq!(offset_of!(MsvcAbsoluteLocator, offset), 4);
        assert_eq!(
            offset_of!(MsvcAbsoluteLocator, construction_displacement),
            8
        );
        #[cfg(target_pointer_width = "32")]
        {
            assert_eq!(size_of::<MsvcAbsoluteLocator>(), 20);
            assert_eq!(offset_of!(MsvcAbsoluteLocator, type_descriptor), 12);
            assert_eq!(offset_of!(MsvcAbsoluteLocator, class_descriptor), 16);
        }
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(size_of::<MsvcAbsoluteLocator>(), 32);
            assert_eq!(offset_of!(MsvcAbsoluteLocator, type_descriptor), 16);
            assert_eq!(offset_of!(MsvcAbsoluteLocator, class_descriptor), 24);
        }
    }

    #[test]
    fn apple_type_names_decode_both_uniqueness_encodings() {
        let original = c"7Example".as_ptr();
        let tagged = original.map_addr(|address| address | (1usize << (usize::BITS - 1)));
        for name in [original, tagged] {
            let decoded = decode_name_pointer(RttiVariant::ItaniumAppleArm64, name);
            assert_eq!(decoded, original);
            // SAFETY: Decoding restores the real static C string's original address.
            assert_eq!(unsafe { CStr::from_ptr(decoded) }, c"7Example");
        }
        let ordinary = decode_name_pointer(RttiVariant::ItaniumPointer, original);
        assert_eq!(ordinary, original);
        // SAFETY: Ordinary Itanium name decoding preserves the static C string pointer.
        assert_eq!(unsafe { CStr::from_ptr(ordinary) }, c"7Example");
    }
}
