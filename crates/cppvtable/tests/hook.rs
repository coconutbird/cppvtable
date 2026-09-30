//! Typed hooks over a declared interface and raw hooks over fake pointer-entry tables.

use core::ffi::c_void;

use cppvtable::hook::{HookMode, RawVtableHook, VtableHook};
use cppvtable::interface;

#[interface(abi = c)]
unsafe trait IPair {
    fn first(&self) -> u32;
    fn second(&self) -> u32;
}

unsafe extern "C" fn one(_this: *mut c_void) -> u32 {
    1
}

unsafe extern "C" fn two(_this: *mut c_void) -> u32 {
    2
}

unsafe extern "C" fn replaced(_this: *mut c_void) -> u32 {
    99
}

fn pair_table() -> IPairVtbl {
    IPairVtbl {
        first: one,
        second: two,
    }
}

/// Call `entry` on a null object; the fixtures never read `this`.
fn call(entry: unsafe extern "C" fn(*mut c_void) -> u32) -> u32 {
    // SAFETY: The fixture functions ignore `this` and have no preconditions.
    unsafe { entry(core::ptr::null_mut()) }
}

#[test]
fn typed_shadow_hook_edits_only_the_object_and_forwards_to_the_original() {
    let mut table = pair_table();
    let native = (&raw mut table).cast_const();
    let mut vptr = native;
    // SAFETY: `vptr` stands in for an object whose only field points at a live table.
    let iface = unsafe { IPair::from_raw((&raw mut vptr).cast()) }.unwrap();
    // SAFETY: The object and its table outlive the hook; C tables have no prefix.
    let mut hook = unsafe { VtableHook::new(&*iface, HookMode::Shadow) };
    assert_eq!(hook.mode(), HookMode::Shadow);
    assert_eq!(hook.entry_count(), 2);
    // SAFETY: `replaced` adds no preconditions and nothing calls through the table.
    unsafe { hook.set(|table| table.second = replaced) };
    assert_eq!(iface.first(), 1);
    assert_eq!(iface.second(), 99);
    assert_eq!(call(hook.original().second), 2);
    drop(hook);
    assert_eq!(vptr, native);
    assert_eq!(call(table.second), 2);
}

#[test]
fn typed_patch_hook_edits_the_native_table_and_restores_it_on_drop() {
    let mut table = pair_table();
    let mut vptr = (&raw mut table).cast_const();
    // SAFETY: `vptr` stands in for an object whose only field points at a live table.
    let iface = unsafe { IPair::from_raw((&raw mut vptr).cast()) }.unwrap();
    // SAFETY: The table is writable, outlives the hook and has no other users.
    let mut hook = unsafe { VtableHook::new(&*iface, HookMode::Patch) };
    // SAFETY: `replaced` adds no preconditions and nothing calls through the table.
    unsafe { hook.set(|table| table.first = replaced) };
    assert_eq!(iface.first(), 99);
    assert_eq!(iface.second(), 2);
    assert_eq!(call(hook.original().first), 1);
    // SAFETY: As for `set`; entry 0 is `first`.
    unsafe { hook.unhook(0) };
    assert_eq!(iface.first(), 1);
    // SAFETY: As for `set`.
    unsafe { hook.set(|table| table.second = replaced) };
    assert_eq!(iface.second(), 99);
    drop(hook);
    assert_eq!(vptr, (&raw const table));
    assert_eq!(call(table.second), 2);
}

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
fn raw_shadow_mode_repoints_only_the_object_and_restores_it_on_drop() {
    let mut table = native_table();
    let native = address_point(&mut table);
    let mut object = native;
    // SAFETY: `object` stands in for a live object; the table is readable and outlives
    // the hook, which is its only user.
    unsafe {
        let mut hook = RawVtableHook::new(as_object(&mut object), PREFIX, 3, HookMode::Shadow);
        assert_eq!(hook.mode(), HookMode::Shadow);
        assert_eq!(hook.entry_count(), 3);
        assert_eq!(object, hook.address_point());
        assert_ne!(object, native);
        assert_eq!(read_table(hook.address_point()), native_table());

        assert_eq!(hook.hook(1, entry(0x999)), entry(0x200));
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
fn raw_patch_mode_writes_the_native_table_and_restores_it_from_the_backup() {
    let mut table = native_table();
    let native = address_point(&mut table);
    let mut object = native;
    // SAFETY: The table is writable and only accessed through `native` until the hook
    // drops; nothing calls through it.
    unsafe {
        let mut hook = RawVtableHook::new(as_object(&mut object), PREFIX, 3, HookMode::Patch);
        assert_eq!(hook.address_point(), native);
        assert_eq!(object, native);

        assert_eq!(hook.hook(0, entry(0x777)), entry(0x100));
        assert_eq!(hook.hook(2, entry(0x999)), entry(0x300));
        assert_eq!(read_table(native), [0x10, 0x20, 0x777, 0x200, 0x999]);
        assert_eq!(hook.original(2), entry(0x300));

        hook.unhook(0);
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
    let hook = unsafe { RawVtableHook::new(as_object(&mut object), 0, 3, HookMode::Shadow) };
    let _ = hook.original(3);
}

#[test]
#[should_panic(expected = "pointer-aligned prefix")]
fn a_partial_prefix_word_is_rejected() {
    let mut table = native_table();
    let mut object = address_point(&mut table);
    // SAFETY: Rejected before the object is modified.
    let _ = unsafe { RawVtableHook::new(as_object(&mut object), 3, 3, HookMode::Shadow) };
}
