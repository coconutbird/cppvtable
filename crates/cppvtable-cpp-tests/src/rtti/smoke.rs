//! RTTI-enabled Rust object dispatch and native runtime casts.

use super::*;
use cppvtable::rtti::{RttiClass, RttiError, RttiMetadata, RttiObject};
use cppvtable::{Object, OwnedObject, implement, interface, vtable_of};
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

fn native_metadata(class: Class) -> [Option<RttiMetadata>; 2] {
    let native = create_native(class);
    // SAFETY: The factory constructs this exact complete class with static native
    // RTTI. Capture its metadata while live; the compiler descriptors outlive it.
    unsafe {
        let primary = RttiMetadata::from_interface(ABI, native.root);
        let secondary = RttiMetadata::from_interface(ABI, native.secondary);
        assert_eq!(primary.complete_object(native.root), native.complete);
        assert_eq!(secondary.complete_object(native.secondary), native.complete);
        assert_eq!(primary.type_info(), secondary.type_info());
        delete_native(native.complete, class);
        [Some(primary), Some(secondary)]
    }
}

fn witness_class() -> RttiClass<RustObject> {
    // SAFETY: Witness has exactly these two nonvirtual interface chains at matching
    // offsets. Native callers use virtual callbacks only and never delete the object.
    unsafe { RttiClass::new(&native_metadata(Class::Witness)) }.unwrap()
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
fn native_rtti_and_rust_callbacks_survive_ownership_transfer() {
    let class = witness_class();
    let drops = Arc::new(AtomicUsize::new(0));
    let owner = RttiObject::new(
        RustObject {
            value: 70,
            drops: drops.clone(),
        },
        &class,
    );
    let raw = owner.into_raw();
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    // SAFETY: Sole ownership returns with the class that created the object.
    let owner = unsafe { RttiObject::from_raw(raw, &class) };
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
        assert_eq!(info.type_info(), type_descriptor(Class::Witness));
        assert!(
            info.mangled_name()
                .to_str()
                .unwrap()
                .contains("CppvtableRttiWitness")
        );
        assert_eq!(info.complete_object(secondary), primary);
        assert_eq!(
            runtime().cast(
                primary,
                type_descriptor(Class::Root),
                type_descriptor(Class::Secondary)
            ),
            secondary
        );
        assert_eq!(
            runtime().cast(
                secondary,
                type_descriptor(Class::Secondary),
                type_descriptor(Class::Witness)
            ),
            primary
        );
        assert!(
            runtime()
                .cast(
                    primary,
                    type_descriptor(Class::Root),
                    type_descriptor(Class::Unrelated)
                )
                .is_null()
        );
        assert!(
            runtime()
                .cast(
                    core::ptr::null_mut(),
                    type_descriptor(Class::Root),
                    type_descriptor(Class::Secondary)
                )
                .is_null()
        );
    }
    drop(owner);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn objects_of_one_class_share_its_rtti_tables() {
    let class = witness_class();
    let drops = Arc::new(AtomicUsize::new(0));
    let first = RttiObject::new(
        RustObject {
            value: 1,
            drops: drops.clone(),
        },
        &class,
    );
    let second = RttiObject::new(
        RustObject {
            value: 2,
            drops: drops.clone(),
        },
        &class,
    );
    let plain = OwnedObject::new(RustObject {
        value: 3,
        drops: drops.clone(),
    });
    let tables = |owner: &OwnedObject<RustObject>| {
        (
            vtable_of(&*owner.interface::<IDerived>()).cast::<c_void>(),
            vtable_of(&*owner.interface::<ISecondary>()).cast::<c_void>(),
        )
    };
    assert_eq!(tables(&first), tables(&second));
    assert_ne!(tables(&first).0, tables(&plain).0);
    assert_ne!(tables(&first).1, tables(&plain).1);
    // SAFETY: Both objects are live and use the Witness metadata.
    unsafe {
        assert!(native_checks(
            first.as_raw::<IDerived>(),
            first.as_raw::<ISecondary>(),
            1
        ));
        assert!(native_checks(
            second.as_raw::<IDerived>(),
            second.as_raw::<ISecondary>(),
            2
        ));
    }
}

#[test]
fn checked_metadata_mismatches_are_rejected() {
    let metadata = native_metadata(Class::Witness);
    let other = native_metadata(Class::OtherWitness);
    // SAFETY: All native descriptors are valid. These structural mismatches are
    // explicitly checked and allowed by RttiClass::new's error contract.
    unsafe {
        assert!(matches!(
            RttiClass::<RustObject>::new(&[]),
            Err(RttiError::InterfaceCount)
        ));
        assert!(matches!(
            RttiClass::<RustObject>::new(&[metadata[1], metadata[0]]),
            Err(RttiError::OffsetMismatch)
        ));
        assert!(matches!(
            RttiClass::<RustObject>::new(&[metadata[0], other[1]]),
            Err(RttiError::TypeMismatch)
        ));
        assert!(matches!(
            RttiClass::<RustObject>::new(&[None, metadata[1]]),
            Err(RttiError::InterfaceKind)
        ));
    }
}
