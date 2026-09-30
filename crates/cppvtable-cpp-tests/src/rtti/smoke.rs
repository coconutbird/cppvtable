//! RTTI-owned Rust object dispatch and native runtime casts.

use super::*;
use cppvtable::rtti::{CppAbi, DynamicCastRuntime, RttiError, RttiMetadata};
use cppvtable::{Object, OwnedObject, RttiOwnedObject, implement, interface};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[interface(abi = cpp)]
unsafe trait IRoot {
    fn root_value(&self) -> i32;
}
#[interface(abi = cpp, extends(IRoot))]
unsafe trait IDerived {
    fn derived_value(&self) -> i32;
}
#[interface(abi = cpp)]
unsafe trait ISecondary {
    fn secondary_value(&self) -> i32;
}

#[implement(IDerived, ISecondary)]
struct RustObject {
    value: i32,
    drops: Arc<AtomicUsize>,
}
impl IRootImpl for RustObject {
    fn root_value(&self) -> i32 {
        self.value
    }
}
impl IDerivedImpl for RustObject {
    fn derived_value(&self) -> i32 {
        self.value + 1
    }
}
impl ISecondaryImpl for RustObject {
    fn secondary_value(&self) -> i32 {
        self.value + 2
    }
}
impl Drop for RustObject {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

const ABI: CppAbi = if cfg!(target_env = "msvc") {
    CppAbi::Msvc
} else {
    CppAbi::Itanium
};

#[cfg(target_env = "msvc")]
unsafe extern "C" {
    fn cppvtable_rtti_runtime_msvc(
        object: *mut c_void,
        delta: i32,
        source: *const c_void,
        target: *const c_void,
        reference: i32,
    ) -> *mut c_void;
}
#[cfg(not(target_env = "msvc"))]
unsafe extern "C" {
    fn cppvtable_rtti_runtime_itanium(
        object: *const c_void,
        source: *const c_void,
        target: *const c_void,
        hint: isize,
    ) -> *mut c_void;
}

pub(super) fn runtime() -> DynamicCastRuntime {
    #[cfg(target_env = "msvc")]
    {
        DynamicCastRuntime::Msvc(cppvtable_rtti_runtime_msvc)
    }
    #[cfg(not(target_env = "msvc"))]
    {
        DynamicCastRuntime::Itanium(cppvtable_rtti_runtime_itanium)
    }
}

fn native_metadata(kind: u32) -> [Option<RttiMetadata>; 2] {
    let native = create_native(kind);
    // SAFETY: The factory constructs this exact complete class with permanent native
    // RTTI. Capture its metadata while live; the compiler descriptors outlive it.
    unsafe {
        let primary = RttiMetadata::from_interface(ABI, native.root);
        let secondary = RttiMetadata::from_interface(ABI, native.secondary);
        assert_eq!(primary.complete_object(native.root), native.complete);
        assert_eq!(secondary.complete_object(native.secondary), native.complete);
        assert_eq!(primary.type_info(), secondary.type_info());
        delete_native(native.complete, kind);
        [Some(primary), Some(secondary)]
    }
}

/// # Safety
/// These pointers must identify the matching live primary and secondary interfaces.
unsafe fn native_checks(primary: *mut c_void, secondary: *mut c_void, value: i32) -> bool {
    cpp!(unsafe [primary as "CppvtableRttiRoot*", secondary as "CppvtableRttiSecondary*", value as "int"] -> bool as "bool" {
        return typeid(*primary) == typeid(CppvtableRttiWitness)
            && typeid(*secondary) == typeid(CppvtableRttiWitness)
            && dynamic_cast<CppvtableRttiWitness*>(primary) == primary
            && dynamic_cast<CppvtableRttiDerived*>(secondary) == primary
            && dynamic_cast<CppvtableRttiSecondary*>(primary) == secondary
            && dynamic_cast<void*>(primary) == primary
            && dynamic_cast<void*>(secondary) == primary
            && dynamic_cast<CppvtableRttiUnrelated*>(primary) == nullptr
            && primary->root_value() == value
            && dynamic_cast<CppvtableRttiDerived*>(primary)->derived_value() == value + 1
            && secondary->secondary_value() == value + 2;
    })
}

#[test]
fn native_rtti_and_rust_callbacks_survive_typed_ownership_transfer() {
    let metadata = native_metadata(0);
    let drops = Arc::new(AtomicUsize::new(0));
    // SAFETY: Witness has exactly these two nonvirtual interface chains at matching
    // offsets. Native callers use virtual callbacks only and never delete the object.
    let owner = unsafe {
        OwnedObject::new_with_rtti(
            RustObject {
                value: 70,
                drops: drops.clone(),
            },
            &metadata,
        )
    }
    .unwrap();
    let raw = owner.into_raw();
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    // SAFETY: Sole ownership is restored with the same RTTI auxiliary allocation type.
    let owner = unsafe { RttiOwnedObject::<RustObject>::from_raw(raw) };
    let primary = owner.as_raw::<IDerived>();
    let secondary = owner.as_raw::<ISecondary>();
    assert_eq!(
        (secondary as usize) - (primary as usize),
        Object::<RustObject>::slot_offset(1)
    );
    // SAFETY: All pointers and static source/target descriptors match this live object.
    unsafe {
        assert!(native_checks(primary, secondary, 70));
        let info = RttiMetadata::from_interface(ABI, secondary);
        assert_eq!(info.type_info(), type_descriptor(3));
        assert!(
            info.mangled_name()
                .to_str()
                .unwrap()
                .contains("CppvtableRttiWitness")
        );
        assert_eq!(info.complete_object(secondary), primary);
        assert_eq!(
            runtime().cast(primary, type_descriptor(0), type_descriptor(2)),
            secondary
        );
        assert_eq!(
            runtime().cast(secondary, type_descriptor(2), type_descriptor(3)),
            primary
        );
        assert!(
            runtime()
                .cast(primary, type_descriptor(0), type_descriptor(4))
                .is_null()
        );
        assert!(
            runtime()
                .cast(
                    core::ptr::null_mut(),
                    type_descriptor(0),
                    type_descriptor(2)
                )
                .is_null()
        );
    }
    drop(owner);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn checked_metadata_mismatches_are_rejected_before_installation() {
    let metadata = native_metadata(0);
    let other = native_metadata(1);
    let drops = Arc::new(AtomicUsize::new(0));
    let value = || RustObject {
        value: 0,
        drops: drops.clone(),
    };
    // SAFETY: All native descriptors are valid. These structural mismatches are
    // explicitly checked and allowed by new_with_rtti's error contract.
    unsafe {
        assert!(matches!(
            OwnedObject::new_with_rtti(value(), &[]),
            Err(RttiError::InterfaceCount)
        ));
        assert!(matches!(
            OwnedObject::new_with_rtti(value(), &[metadata[1], metadata[0]]),
            Err(RttiError::OffsetMismatch)
        ));
        assert!(matches!(
            OwnedObject::new_with_rtti(value(), &[metadata[0], other[1]]),
            Err(RttiError::TypeMismatch)
        ));
        assert!(matches!(
            OwnedObject::new_with_rtti(value(), &[None, metadata[1]]),
            Err(RttiError::InterfaceKind)
        ));
    }
    assert_eq!(drops.load(Ordering::Relaxed), 4);
}
