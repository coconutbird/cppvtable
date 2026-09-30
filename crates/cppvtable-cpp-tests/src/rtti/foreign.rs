//! Native inheritance access rules and leaf/single-inheritance RTTI kinds.

use super::*;
use cppvtable::rtti::{CppAbi, RttiMetadata};
use cppvtable::{OwnedObject, implement, interface};

const ABI: CppAbi = if cfg!(target_env = "msvc") {
    CppAbi::Msvc
} else {
    CppAbi::Itanium
};

#[test]
fn foreign_virtual_ambiguous_and_private_bases_follow_native_cast_rules() {
    let runtime = super::smoke::runtime();
    for (kind, descriptor) in [(2, 6), (3, 9), (4, 5)] {
        let native = create_native(kind);
        // SAFETY: The exact factory class is fully constructed and remains alive;
        // its compiler-produced hierarchy and native runtime match these descriptors.
        unsafe {
            let root = RttiMetadata::from_interface(ABI, native.root);
            let side = RttiMetadata::from_interface(ABI, native.secondary);
            assert_eq!(root.type_info(), type_descriptor(descriptor));
            assert_eq!(side.type_info(), root.type_info());
            assert_eq!(root.complete_object(native.root), native.complete);
            assert_eq!(side.complete_object(native.secondary), native.complete);
            assert_eq!(
                runtime.cast(
                    native.secondary,
                    type_descriptor(2),
                    type_descriptor(descriptor)
                ),
                native.complete
            );
            let root_from_side =
                runtime.cast(native.secondary, type_descriptor(2), type_descriptor(0));
            let complete_from_root =
                runtime.cast(native.root, type_descriptor(0), type_descriptor(descriptor));
            if kind == 2 {
                assert_eq!(root_from_side, native.root);
            } else {
                // The target base is ambiguous or private, as encoded by native RTTI.
                assert!(root_from_side.is_null());
            }
            if kind == 4 {
                assert!(complete_from_root.is_null());
            } else {
                assert_eq!(complete_from_root, native.complete);
            }
            delete_native(native.complete, kind);
            assert!(!root.mangled_name().is_empty());
        }
    }
}

/// # Safety
/// `root` must refer to a live Witness with its declared nonvirtual interface layout.
unsafe fn reference_casts(root: *mut c_void) -> bool {
    cpp!(unsafe [root as "CppvtableRttiRoot*"] -> bool as "bool" {
        auto& complete = dynamic_cast<CppvtableRttiWitness&>(*root);
        if (static_cast<CppvtableRttiRoot*>(&complete) != root) return false;
        try {
            (void)dynamic_cast<CppvtableRttiUnrelated&>(*root);
            return false;
        } catch (const std::bad_cast&) {
            return true;
        }
    })
}

#[test]
fn native_reference_cast_failure_is_caught_before_returning_to_rust() {
    let native = create_native(0);
    // SAFETY: The matching native object remains alive; all C++ exceptions are caught.
    unsafe {
        assert!(reference_casts(native.root));
        delete_native(native.complete, 0);
    }
}

#[interface(abi = cpp)]
unsafe trait ILeaf {
    fn leaf_value(&self) -> i32;
}
#[interface(abi = cpp, extends(ILeaf))]
unsafe trait ISingle {
    fn single_value(&self) -> i32;
}

#[implement(ILeaf)]
struct Leaf {
    value: i32,
}
impl ILeafImpl for Leaf {
    fn leaf_value(&self) -> i32 {
        self.value
    }
}

#[implement(ISingle)]
struct Single {
    value: i32,
}
impl ILeafImpl for Single {
    fn leaf_value(&self) -> i32 {
        self.value
    }
}
impl ISingleImpl for Single {
    fn single_value(&self) -> i32 {
        self.value + 1
    }
}

fn metadata_for(kind: u32) -> RttiMetadata {
    let native = create_native(kind);
    // SAFETY: Capture permanent native descriptors before releasing the concrete instance.
    unsafe {
        let metadata = RttiMetadata::from_interface(ABI, native.root);
        delete_native(native.complete, kind);
        metadata
    }
}

/// # Safety
/// `object` must match the Leaf/Single class selected by `single` and remain live.
unsafe fn leaf_checks(object: *mut c_void, single: bool, value: i32) -> bool {
    cpp!(unsafe [object as "CppvtableRttiLeaf*", single as "bool", value as "int"] -> bool as "bool" {
        if (object->leaf_value() != value) return false;
        auto* child = dynamic_cast<CppvtableRttiSingle*>(object);
        if (single) {
            return typeid(*object) == typeid(CppvtableRttiSingle)
                && child != nullptr && child->single_value() == value + 1;
        }
        return typeid(*object) == typeid(CppvtableRttiLeaf) && child == nullptr;
    })
}

#[test]
fn leaf_and_single_inheritance_type_descriptors_work_for_rust_objects() {
    let leaf = metadata_for(5);
    let single = metadata_for(6);
    assert_eq!(leaf.type_info(), type_descriptor(7));
    assert_eq!(single.type_info(), type_descriptor(8));
    // SAFETY: Each matching native class is a single nonvirtual interface chain at
    // offset zero. Callbacks remain virtual and the native metadata is permanent.
    unsafe {
        let leaf_owner = OwnedObject::new_with_rtti(Leaf { value: 101 }, &[Some(leaf)]).unwrap();
        let single_owner =
            OwnedObject::new_with_rtti(Single { value: 201 }, &[Some(single)]).unwrap();
        assert!(leaf_checks(leaf_owner.as_raw::<ILeaf>(), false, 101));
        assert!(leaf_checks(single_owner.as_raw::<ISingle>(), true, 201));
    }
}
