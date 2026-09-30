//! Native inheritance access rules and leaf/single-inheritance RTTI kinds.

use super::*;
use cppvtable::rtti::{CppAbi, DynamicCastRuntime, RttiClass, RttiMetadata, RttiObject};
use cppvtable::{implement, interface};

#[test]
fn foreign_virtual_ambiguous_and_private_bases_follow_native_cast_rules() {
    let runtime = DynamicCastRuntime::TARGET;
    for class in [
        Class::VirtualWitness,
        Class::AmbiguousWitness,
        Class::PrivateWitness,
    ] {
        let native = create_native(class);
        // SAFETY: The exact factory class is fully constructed and remains alive;
        // its compiler-produced hierarchy and native runtime match these descriptors.
        unsafe {
            let root = RttiMetadata::from_interface(CppAbi::TARGET, native.root);
            let side = RttiMetadata::from_interface(CppAbi::TARGET, native.secondary);
            assert_eq!(root.type_info(), type_descriptor(class));
            assert_eq!(side.type_info(), root.type_info());
            assert_eq!(root.complete_object(native.root), native.complete);
            assert_eq!(side.complete_object(native.secondary), native.complete);
            assert_eq!(
                runtime.cast(
                    native.secondary,
                    type_descriptor(Class::Secondary),
                    type_descriptor(class)
                ),
                native.complete
            );
            let root_from_side = runtime.cast(
                native.secondary,
                type_descriptor(Class::Secondary),
                type_descriptor(Class::Root),
            );
            let complete_from_root = runtime.cast(
                native.root,
                type_descriptor(Class::Root),
                type_descriptor(class),
            );
            match class {
                Class::VirtualWitness => assert_eq!(root_from_side, native.root),
                // The target base is ambiguous or private, as encoded by native RTTI.
                _ => assert!(root_from_side.is_null()),
            }
            match class {
                Class::PrivateWitness => assert!(complete_from_root.is_null()),
                _ => assert_eq!(complete_from_root, native.complete),
            }
            assert!(!root.mangled_name().is_empty());
            delete_native(native.complete, class);
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
    let native = create_native(Class::Witness);
    // SAFETY: The matching native object remains alive; all C++ exceptions are caught.
    unsafe {
        assert!(reference_casts(native.root));
        delete_native(native.complete, Class::Witness);
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

fn metadata_for(class: Class) -> RttiMetadata {
    let native = create_native(class);
    // SAFETY: Leaf and Single both start with the Leaf chain declared by `ILeaf`.
    // Capture static native descriptors before releasing the concrete instance.
    unsafe {
        let metadata = {
            let leaf = ILeaf::from_raw(native.root).expect("factory allocation succeeded");
            RttiMetadata::of(&*leaf)
        };
        delete_native(native.complete, class);
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
    let leaf_metadata = metadata_for(Class::Leaf);
    let single_metadata = metadata_for(Class::Single);
    assert_eq!(leaf_metadata.type_info(), type_descriptor(Class::Leaf));
    assert_eq!(single_metadata.type_info(), type_descriptor(Class::Single));
    // SAFETY: Each matching native class is a single nonvirtual interface chain at
    // offset zero. Callbacks remain virtual and the native metadata stays loaded.
    let (leaf_class, single_class) = unsafe {
        (
            RttiClass::<Leaf>::builder()
                .with::<ILeaf>(leaf_metadata)
                .build()
                .unwrap(),
            RttiClass::<Single>::builder()
                .with::<ISingle>(single_metadata)
                .build()
                .unwrap(),
        )
    };
    let leaf = RttiObject::new(Leaf { value: 101 }, &leaf_class);
    let single = RttiObject::new(Single { value: 201 }, &single_class);
    // SAFETY: Both objects are live and built from their matching native classes.
    unsafe {
        assert!(leaf_checks(leaf.as_raw::<ILeaf>(), false, 101));
        assert!(leaf_checks(single.as_raw::<ISingle>(), true, 201));
    }
}
