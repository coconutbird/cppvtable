//! Vtable hooks in both modes over fake pointer-entry tables.

use core::ffi::c_void;

use cppvtable::hook::{HookMode, VtableHook};

/// Two prefix words followed by three entries; the address point is index 2.
fn native_table() -> [usize; 5] {
    [0x10, 0x20, 0x100, 0x200, 0x300]
}

const PREFIX: usize = 2 * size_of::<usize>();

fn address_point(table: &mut [usize; 5]) -> *const c_void {
    table.as_mut_ptr().wrapping_add(2).cast_const().cast()
}

fn entry(value: usize) -> *const c_void {
    value as *const c_void
}

/// A stand-in object whose only field is its vtable pointer.
fn as_object(vptr: &mut *const c_void) -> *mut c_void {
    core::ptr::from_mut(vptr).cast()
}

/// # Safety
/// `address_point` must be preceded by two words and followed by three entries.
unsafe fn read_table(address_point: *const c_void) -> [usize; 5] {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        address_point
            .cast::<usize>()
            .sub(2)
            .cast::<[usize; 5]>()
            .read()
    }
}

#[test]
fn shadow_mode_repoints_only_the_object_and_restores_it_on_drop() {
    let mut table = native_table();
    let native = address_point(&mut table);
    let mut object = native;
    // SAFETY: `object` stands in for a live object; the table is readable and outlives
    // the hook, which is its only user.
    unsafe {
        let mut hook = VtableHook::new(as_object(&mut object), PREFIX, 3, HookMode::Shadow);
        assert_eq!(hook.mode(), HookMode::Shadow);
        assert_eq!(hook.entry_count(), 3);
        assert_eq!(object, hook.address_point());
        assert_ne!(object, native);
        assert_eq!(read_table(hook.address_point()), native_table());

        assert_eq!(hook.replace(1, entry(0x999)), entry(0x200));
        assert_eq!(
            read_table(hook.address_point()),
            [0x10, 0x20, 0x100, 0x999, 0x300]
        );
        assert_eq!(hook.original(1), entry(0x200));
        assert_eq!(read_table(native), native_table());

        drop(hook);
    }
    assert_eq!(object, native);
}

#[test]
fn patch_mode_writes_the_native_table_and_restores_it_from_the_backup() {
    let mut table = native_table();
    let native = address_point(&mut table);
    let mut object = native;
    // SAFETY: The table is writable and only accessed through `native` until the hook
    // drops; nothing calls through it.
    unsafe {
        let mut hook = VtableHook::new(as_object(&mut object), PREFIX, 3, HookMode::Patch);
        assert_eq!(hook.address_point(), native);
        assert_eq!(object, native);

        assert_eq!(hook.replace(0, entry(0x777)), entry(0x100));
        assert_eq!(hook.replace(2, entry(0x999)), entry(0x300));
        assert_eq!(read_table(native), [0x10, 0x20, 0x777, 0x200, 0x999]);
        assert_eq!(hook.original(2), entry(0x300));

        hook.restore(0);
        assert_eq!(read_table(native), [0x10, 0x20, 0x100, 0x200, 0x999]);

        drop(hook);
    }
    assert_eq!(object, native);
    assert_eq!(table, native_table());
}

#[test]
#[should_panic(expected = "vtable slot out of range")]
fn slots_past_the_copied_count_are_rejected() {
    let mut table = native_table();
    let mut object = address_point(&mut table);
    // SAFETY: The table is readable and outlives the hook.
    let hook = unsafe { VtableHook::new(as_object(&mut object), 0, 3, HookMode::Shadow) };
    let _ = hook.original(3);
}

#[test]
#[should_panic(expected = "pointer-aligned prefix")]
fn a_partial_prefix_word_is_rejected() {
    let mut table = native_table();
    let mut object = address_point(&mut table);
    // SAFETY: Rejected before the object is modified.
    let _ = unsafe { VtableHook::new(as_object(&mut object), 3, 3, HookMode::Shadow) };
}
