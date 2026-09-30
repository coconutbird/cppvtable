//! Vtable hooking of native C++ objects that keeps native RTTI intact.
//!
//! Compiler vtables are read-only, so patch mode is exercised on a shadow copy: a
//! patch hook stacked on a shadow-hooked object overwrites that object's copy.

use super::*;
use cppvtable::hook::{HookMode, VtableHook};
use cppvtable::rtti::RttiMetadata;
use std::sync::atomic::{AtomicPtr, Ordering};

static ORIGINAL_ROOT: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

macro_rules! member {
    ($(fn $name:ident($this:ident: *mut c_void) -> i32 $body:block)*) => {$(
        #[cfg(all(target_arch = "x86", target_os = "windows"))]
        unsafe extern "thiscall" fn $name($this: *mut c_void) -> i32 $body
        #[cfg(not(all(target_arch = "x86", target_os = "windows")))]
        unsafe extern "C" fn $name($this: *mut c_void) -> i32 $body
    )*};
}

#[cfg(all(target_arch = "x86", target_os = "windows"))]
type Method = unsafe extern "thiscall" fn(*mut c_void) -> i32;
#[cfg(not(all(target_arch = "x86", target_os = "windows")))]
type Method = unsafe extern "C" fn(*mut c_void) -> i32;

member! {
    fn hooked_root(this: *mut c_void) -> i32 {
        // SAFETY: The stored entry is the native `root_value` with this signature.
        let original: Method = unsafe { core::mem::transmute(ORIGINAL_ROOT.load(Ordering::Relaxed)) };
        // SAFETY: `this` is the hooked live object passed by the native caller.
        unsafe { original(this) + 100 }
    }
    fn patched_derived(_this: *mut c_void) -> i32 {
        -1
    }
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
    // `derived_value`, which the hooks implement with the native signature. The
    // patched table is the writable shadow copy. No other thread touches these objects.
    unsafe {
        let metadata = RttiMetadata::from_interface(ABI, hooked.root);
        let mut shadow = VtableHook::new(hooked.root, metadata.prefix_size(), 2, HookMode::Shadow);
        ORIGINAL_ROOT.store(shadow.original(0).cast_mut(), Ordering::Relaxed);
        let _ = shadow.replace(0, hooked_root as *const c_void);
        assert_eq!(values(hooked.root), (111, 22));
        assert_eq!(values(other.root), (11, 22));
        assert!(rtti_intact(hooked.root, hooked.secondary));
        assert_eq!(
            RttiMetadata::from_interface(ABI, hooked.root).type_info(),
            type_descriptor(Class::Witness)
        );

        let mut patch = VtableHook::new(hooked.root, metadata.prefix_size(), 2, HookMode::Patch);
        assert_eq!(patch.address_point(), shadow.address_point());
        let _ = patch.replace(1, patched_derived as *const c_void);
        assert_eq!(values(hooked.root), (111, -1));
        assert_eq!(values(other.root), (11, 22));
        drop(patch);
        assert_eq!(values(hooked.root), (111, 22));

        drop(shadow);
        assert_eq!(values(hooked.root), (11, 22));
        assert_eq!(values(other.root), (11, 22));
        assert!(rtti_intact(hooked.root, hooked.secondary));
        delete_native(hooked.complete, Class::Witness);
        delete_native(other.complete, Class::Witness);
    }
}
