//! Vtable copies used for per-object hooking.

use core::ffi::c_void;

use cppvtable::hook::{ShadowVtable, swap_vtable};

/// Two prefix words followed by three entries; the address point is index 2.
fn native_table() -> [usize; 5] {
    [0x10, 0x20, 0x100, 0x200, 0x300]
}

fn address_point(table: &[usize; 5]) -> *const c_void {
    core::ptr::from_ref(&table[2]).cast()
}

#[test]
fn a_copy_keeps_the_prefix_and_entries_and_replaces_without_touching_the_source() {
    let source = native_table();
    let prefix = 2 * size_of::<usize>();
    // SAFETY: The prefix and three entries are readable.
    let mut shadow = unsafe { ShadowVtable::copy_native(address_point(&source), prefix, 3) };
    assert_eq!(shadow.entry_count(), 3);
    let copied = shadow.address_point().cast::<usize>();
    // SAFETY: The copy holds the two prefix words before its address point.
    assert_eq!(
        unsafe { [copied.sub(2).read(), copied.sub(1).read()] },
        [0x10, 0x20]
    );
    assert_eq!(shadow.entry(2), 0x300 as *const c_void);

    // SAFETY: No object uses the copy yet.
    let previous = unsafe { shadow.replace(1, 0x999 as *const c_void) };
    assert_eq!(previous, 0x200 as *const c_void);
    assert_eq!(shadow.entry(1), 0x999 as *const c_void);
    assert_eq!(source, native_table());
}

#[test]
fn swapping_returns_the_previous_address_point_for_restoration() {
    let source = native_table();
    // SAFETY: The prefix and entries are readable.
    let shadow = unsafe { ShadowVtable::copy_native(address_point(&source), 0, 3) };
    let mut object = address_point(&source);
    let raw = core::ptr::from_mut(&mut object).cast::<c_void>();
    // SAFETY: `object` stands in for a live object whose first field is its vptr.
    unsafe {
        assert_eq!(
            swap_vtable(raw, shadow.address_point()),
            address_point(&source)
        );
        assert_eq!(object, shadow.address_point());
        assert_eq!(
            swap_vtable(raw, address_point(&source)),
            shadow.address_point()
        );
    }
    assert_eq!(object, address_point(&source));
}

#[test]
#[should_panic(expected = "vtable slot out of range")]
fn entries_past_the_copied_count_are_rejected() {
    let source = native_table();
    // SAFETY: The entries are readable.
    let shadow = unsafe { ShadowVtable::copy_native(address_point(&source), 0, 3) };
    let _ = shadow.entry(3);
}

#[test]
#[should_panic(expected = "pointer-aligned prefix")]
fn a_partial_prefix_word_is_rejected() {
    let source = native_table();
    // SAFETY: Rejected before reading.
    let _ = unsafe { ShadowVtable::copy_native(address_point(&source), 3, 3) };
}
