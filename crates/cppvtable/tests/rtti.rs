//! RTTI class validation and header installation, using synthetic native metadata.
//!
//! The synthetic descriptors are only compared by address and never dereferenced;
//! nothing here hands an object to native RTTI operations.

use core::ffi::c_void;
use core::ptr;

use cppvtable::rtti::{
    CppAbi, ItaniumPrefix, MsvcAbsoluteLocator, MsvcPrefix, RttiClass, RttiError, RttiMetadata,
    RttiObject, RttiVariant,
};
use cppvtable::{Object, OwnedObject, implement, interface};
use cppvtable_abi::interface::vtable_of;

#[interface(abi = c)]
unsafe trait IPlain {
    fn plain(&self) -> u32;
}

#[interface(abi = cpp)]
unsafe trait IValue {
    fn value(&self) -> u32;
}

#[implement(IPlain, IValue)]
struct Mixed {
    value: u32,
}

impl IPlainImpl for Mixed {
    fn plain(&self) -> u32 {
        self.value
    }
}

impl IValueImpl for Mixed {
    fn value(&self) -> u32 {
        self.value + 1
    }
}

#[implement(IValue)]
struct Single {
    value: u32,
}

impl IValueImpl for Single {
    fn value(&self) -> u32 {
        self.value
    }
}

/// Stand-ins for native type descriptors; only their addresses are used.
static DESCRIPTOR: [usize; 4] = [0; 4];
static HIERARCHY: [u32; 4] = [0; 4];
#[cfg(target_env = "msvc")]
static VIRTUAL_HIERARCHY: [u32; 4] = [0, 2, 0, 0];

fn descriptor() -> *const c_void {
    ptr::from_ref(&DESCRIPTOR).cast()
}

fn locator(offset: usize, construction: u32, hierarchy: &'static [u32; 4]) -> MsvcAbsoluteLocator {
    MsvcAbsoluteLocator {
        signature: 0,
        offset: offset.try_into().unwrap(),
        construction_displacement: construction,
        type_descriptor: descriptor(),
        class_descriptor: ptr::from_ref(hierarchy).cast(),
    }
}

/// # Safety
/// `locator` must outlive every use of the returned metadata.
unsafe fn msvc(locator: &MsvcAbsoluteLocator) -> RttiMetadata {
    let prefix = MsvcPrefix {
        locator: ptr::from_ref(locator).cast(),
    };
    // SAFETY: The caller keeps the locator alive; its descriptors are never read.
    unsafe { RttiMetadata::from_msvc_prefix_variant(RttiVariant::MsvcAbsolute, prefix) }
}

fn itanium(offset: usize) -> RttiMetadata {
    let prefix = ItaniumPrefix {
        offset_to_top: -isize::try_from(offset).unwrap(),
        type_info: descriptor(),
    };
    // SAFETY: The descriptor address is only compared, never dereferenced.
    unsafe { RttiMetadata::from_itanium_prefix_variant(RttiVariant::ItaniumPointer, prefix) }
}

/// Metadata for `abi`, reading `locator` only when it is Microsoft.
///
/// # Safety
/// `locator` must outlive every use of the returned metadata.
unsafe fn metadata_for(abi: CppAbi, offset: usize, locator: &MsvcAbsoluteLocator) -> RttiMetadata {
    match abi {
        // SAFETY: Guaranteed by the caller.
        CppAbi::Msvc => unsafe { msvc(locator) },
        CppAbi::Itanium => itanium(offset),
    }
}

#[test]
fn mixed_c_and_cpp_interfaces_install_rtti_only_on_the_cpp_header() {
    let offset = Object::<Mixed>::slot_offset(1);
    let msvc_locator = locator(offset, 0, &HIERARCHY);
    // SAFETY: The locator outlives the class and every object below.
    let metadata = unsafe { metadata_for(CppAbi::TARGET, offset, &msvc_locator) };
    // SAFETY: Synthetic metadata matches the Rust layout; no native code uses RTTI.
    let class = unsafe {
        RttiClass::<Mixed>::builder()
            .with::<IValue>(metadata)
            .build()
    }
    .unwrap();
    let object = RttiObject::new(Mixed { value: 5 }, &class);
    let plain = OwnedObject::new(Mixed { value: 0 });

    assert_eq!(object.interface::<IPlain>().plain(), 5);
    assert_eq!(object.interface::<IValue>().value(), 6);
    assert!(ptr::eq(
        vtable_of(&*object.interface::<IPlain>()),
        vtable_of(&*plain.interface::<IPlain>())
    ));
    assert!(!ptr::eq(
        vtable_of(&*object.interface::<IValue>()),
        vtable_of(&*plain.interface::<IValue>())
    ));

    let value = object.as_raw::<IValue>();
    // SAFETY: The installed prefix copies the synthetic metadata above.
    let installed = unsafe { RttiMetadata::from_interface_variant(metadata.variant(), value) };
    assert_eq!(installed.type_info(), descriptor());
    assert_eq!(installed.offset_to_top(), -isize::try_from(offset).unwrap());
    // SAFETY: `value` is a live interface described by the installed metadata.
    let complete = unsafe { installed.complete_object(value) };
    assert_eq!(complete, object.as_raw::<IPlain>());
}

#[test]
fn a_cpp_interface_without_metadata_is_rejected() {
    // SAFETY: Rejected before any table is built.
    let result = unsafe { RttiClass::<Mixed>::builder().build() };
    assert!(matches!(result, Err(RttiError::InterfaceKind)));
}

#[test]
fn metadata_from_the_other_cpp_abi_is_rejected() {
    let msvc_locator = locator(0, 0, &HIERARCHY);
    let other = match CppAbi::TARGET {
        CppAbi::Msvc => CppAbi::Itanium,
        CppAbi::Itanium => CppAbi::Msvc,
    };
    // SAFETY: The locator outlives the metadata.
    let foreign = unsafe { metadata_for(other, 0, &msvc_locator) };
    // SAFETY: Rejected before any table is built.
    let result = unsafe {
        RttiClass::<Single>::builder()
            .with::<IValue>(foreign)
            .build()
    };
    assert!(matches!(result, Err(RttiError::AbiMismatch)));
}

#[cfg(target_env = "msvc")]
#[test]
fn microsoft_construction_and_virtual_inheritance_metadata_is_rejected() {
    let construction = locator(0, 4, &HIERARCHY);
    let virtual_base = locator(0, 0, &VIRTUAL_HIERARCHY);
    // SAFETY: The locators and hierarchy flags outlive the metadata. Both
    // mismatches are rejected before any table is built.
    unsafe {
        assert!(matches!(
            RttiClass::<Single>::builder()
                .with::<IValue>(msvc(&construction))
                .build(),
            Err(RttiError::ConstructionTable)
        ));
        assert!(matches!(
            RttiClass::<Single>::builder()
                .with::<IValue>(msvc(&virtual_base))
                .build(),
            Err(RttiError::VirtualInheritance)
        ));
    }
}
