//! Per-function conventions and a partially declared native C vtable.

use cppvtable::{OwnedObject, implement, interface};
use std::ffi::c_void;

#[interface(abi = c)]
unsafe trait IConventions {
    #[abi(convention = "cdecl")]
    fn plain(&self, value: i32) -> i32;
    #[abi(convention = "stdcall")]
    fn system_call(&self, value: i32) -> i32;
    #[abi(convention = "fastcall")]
    fn fast_call(&self, value: i32) -> i32;
}

#[implement(IConventions)]
struct Conventions {
    value: i32,
}
impl IConventionsImpl for Conventions {
    fn plain(&self, value: i32) -> i32 {
        self.value + value
    }
    fn system_call(&self, value: i32) -> i32 {
        self.value + 2 * value
    }
    fn fast_call(&self, value: i32) -> i32 {
        self.value + 3 * value
    }
}

#[interface(abi = c, slots = 50)]
unsafe trait IPartial {
    #[slot(32)]
    fn known(&self, value: i32) -> i32;
}

#[interface(abi = c)]
unsafe trait IPartialPrefix {
    #[slot(32)]
    fn known(&self, value: i32) -> i32;
}

#[implement(IPartial)]
struct Partial {
    value: i32,
}
impl IPartialImpl for Partial {
    fn known(&self, value: i32) -> i32 {
        self.value + value
    }
}

unsafe extern "C" {
    fn cppvtable_c_delete(object: *mut c_void);
    fn cppvtable_c_conventions_create(value: i32) -> *mut c_void;
    fn cppvtable_c_conventions_cdecl(object: *mut c_void, value: i32) -> i32;
    fn cppvtable_c_conventions_stdcall(object: *mut c_void, value: i32) -> i32;
    fn cppvtable_c_conventions_fastcall(object: *mut c_void, value: i32) -> i32;
    fn cppvtable_c_partial_create(value: i32) -> *mut c_void;
    fn cppvtable_c_partial_call(object: *mut c_void, value: i32) -> i32;
}

#[test]
fn rust_calls_c_per_function_conventions() {
    // SAFETY: C allocates the matching object and it lives through all calls.
    let raw = unsafe { cppvtable_c_conventions_create(10) };
    {
        // SAFETY: The C object lives until the delete below, after the last borrow.
        let interface = unsafe { IConventions::from_raw(raw) }.expect("C allocation succeeded");
        assert_eq!(interface.plain(7), 17);
        assert_eq!(interface.system_call(7), 24);
        assert_eq!(interface.fast_call(7), 31);
    }
    // SAFETY: The C allocation is deleted once, after its last borrow.
    unsafe { cppvtable_c_delete(raw) };
}

#[test]
fn c_calls_rust_per_function_conventions() {
    let owner = OwnedObject::new(Conventions { value: 10 });
    let raw = owner.as_raw::<IConventions>();
    // SAFETY: The owner keeps its matching table alive during every native call.
    unsafe {
        assert_eq!(cppvtable_c_conventions_cdecl(raw, 7), 17);
        assert_eq!(cppvtable_c_conventions_stdcall(raw, 7), 24);
        assert_eq!(cppvtable_c_conventions_fastcall(raw, 7), 31);
    }
}

#[test]
fn rust_calls_known_entry_in_native_partial_table() {
    assert_eq!(
        std::mem::size_of::<IPartialVtbl>(),
        50 * std::mem::size_of::<usize>()
    );
    assert_eq!(
        std::mem::size_of::<IPartialPrefixVtbl>(),
        33 * std::mem::size_of::<usize>()
    );
    assert_eq!(
        std::mem::offset_of!(IPartialVtbl, known),
        32 * std::mem::size_of::<usize>()
    );
    // SAFETY: C allocates the 50-entry table, with the matching known function at entry 32.
    let raw = unsafe { cppvtable_c_partial_create(10) };
    // SAFETY: The C object lives until the delete below, after the last borrow.
    let full = unsafe { IPartial::from_raw(raw) }.expect("C allocation succeeded");
    assert_eq!(full.known(7), 17);
    // SAFETY: A prefix view of the same live table.
    let prefix = unsafe { IPartialPrefix::from_raw(raw) }.expect("C allocation succeeded");
    assert_eq!(prefix.known(7), 17);
    // SAFETY: The C allocation is deleted once, after its last borrow.
    unsafe { cppvtable_c_delete(raw) };
}

#[test]
fn c_calls_known_entry_in_rust_partial_table() {
    let owner = OwnedObject::new(Partial { value: 10 });
    // SAFETY: Only declared entry 32 is called through the owner's matching table.
    assert_eq!(
        unsafe { cppvtable_c_partial_call(owner.as_raw::<IPartial>(), 7) },
        17
    );
}
