//! Vtable hooking of native C++ objects that keeps native RTTI intact.
//!
//! Compiler vtables are read-only, so patch mode is exercised on a shadow copy: a
//! patch hook stacked on a shadow-hooked object overwrites that object's copy.

use super::*;
use cppvtable::hook::HookMode;
use cppvtable::interface;
use cppvtable::rtti::RttiMetadata;
use std::sync::OnceLock;

/// The primary chain of `CppvtableRttiWitness`, flattened to its two entries.
#[interface(abi = cpp)]
unsafe trait IWitness {
    fn root_value(&self) -> i32;
    fn derived_value(&self) -> i32;
}

/// The native Witness table, captured before hooking for forwarding.
static ORIGINAL: OnceLock<IWitnessVtbl> = OnceLock::new();

#[cppvtable::vtable_fn(abi = cpp)]
unsafe fn hooked_root(this: *mut c_void) -> i32 {
    let original = ORIGINAL.get().expect("captured before hooking").root_value;
    // SAFETY: `original` is the native `root_value` of the live Witness `this`.
    unsafe { original(this) + 100 }
}

#[cppvtable::vtable_fn(abi = cpp)]
unsafe fn patched_derived(_this: *mut c_void) -> i32 {
    -1
}

/// # Safety
/// `root` must be the live primary interface of a Witness.
unsafe fn values(root: *mut c_void) -> (i32, i32) {
    let root_value = cpp!(unsafe [root as "CppvtableRttiDerived*"] -> i32 as "int" {
        return root->root_value();
    });
    let derived_value = cpp!(unsafe [root as "CppvtableRttiDerived*"] -> i32 as "int" {
        return root->derived_value();
    });
    (root_value, derived_value)
}

/// # Safety
/// The pointers must be the live primary and secondary interfaces of one Witness.
unsafe fn rtti_intact(root: *mut c_void, secondary: *mut c_void) -> bool {
    cpp!(unsafe [root as "CppvtableRttiDerived*", secondary as "CppvtableRttiSecondary*"] -> bool as "bool" {
        return typeid(*root) == typeid(CppvtableRttiWitness)
            && dynamic_cast<CppvtableRttiSecondary*>(root) == secondary
            && dynamic_cast<CppvtableRttiDerived*>(secondary) == root
            && dynamic_cast<void*>(root) == root;
    })
}

#[test]
fn stacked_hooks_affect_one_object_and_keep_native_rtti() {
    let hooked = create_native(Class::Witness);
    let other = create_native(Class::Witness);
    // SAFETY: Both Witness objects stay alive until deleted below, after both hooks
    // drop in reverse order. Witness's primary table has exactly `root_value` and
    // `derived_value`, which the replacements implement with the declared signature.
    // The patched table is the writable shadow copy. No other thread touches these
    // objects, and no table borrow is held across an edit.
    unsafe {
        let root = IWitness::from_raw(hooked.root).expect("factory allocation succeeded");
        let table = |iface: &IWitness| std::ptr::from_ref(iface.vtable());

        let mut shadow = root.hook(HookMode::Shadow);
        ORIGINAL.get_or_init(|| *shadow.original());
        shadow.set(|t| t.root_value = hooked_root);
        let shadow_table = table(&root);
        assert_eq!(values(hooked.root), (111, 22));
        assert_eq!(values(other.root), (11, 22));
        assert!(rtti_intact(hooked.root, hooked.secondary));
        assert_eq!(
            RttiMetadata::of(&*root).type_info(),
            type_descriptor(Class::Witness)
        );

        let mut patch = root.hook(HookMode::Patch);
        patch.set(|t| t.derived_value = patched_derived);
        assert_eq!(table(&root), shadow_table);
        assert_eq!(values(hooked.root), (111, -1));
        assert_eq!(values(other.root), (11, 22));
        drop(patch);
        assert_eq!(values(hooked.root), (111, 22));

        drop(shadow);
        assert_ne!(table(&root), shadow_table);
        assert_eq!(values(hooked.root), (11, 22));
        assert_eq!(values(other.root), (11, 22));
        assert!(rtti_intact(hooked.root, hooked.secondary));
        delete_native(hooked.complete, Class::Witness);
        delete_native(other.complete, Class::Witness);
    }
}
