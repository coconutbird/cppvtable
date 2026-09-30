//! Bidirectional calls through a table compiled by a C compiler.

use cppvtable::{OwnedObject, implement, interface};
use std::cell::Cell;
use std::ffi::c_void;

#[interface(abi = c)]
unsafe trait ICValue {
    fn get(&self) -> i64;
    fn set(&self, value: i64);
    /// Read a value from the caller's storage.
    ///
    /// # Safety
    ///
    /// `input` must point to an initialized, aligned `i64` readable for this call.
    unsafe fn read(&self, input: *const i64) -> i64;
    /// Copy the stored value into the caller's storage.
    ///
    /// # Safety
    ///
    /// `output` must point to an aligned `i64` writable for this call.
    unsafe fn write(&self, output: *mut i64);
}

#[implement(ICValue)]
struct Value {
    value: Cell<i64>,
}

impl ICValueImpl for Value {
    fn get(&self) -> i64 {
        self.value.get()
    }
    fn set(&self, value: i64) {
        self.value.set(value);
    }
    unsafe fn read(&self, input: *const i64) -> i64 {
        // SAFETY: The implementation contract requires initialized readable storage.
        unsafe { *input }
    }
    unsafe fn write(&self, output: *mut i64) {
        // SAFETY: The implementation contract requires aligned writable storage.
        unsafe { output.write(self.value.get()) };
    }
}

unsafe extern "C" {
    fn cppvtable_c_create(value: i64) -> *mut c_void;
    fn cppvtable_c_delete(object: *mut c_void);
    fn cppvtable_c_get(object: *mut c_void) -> i64;
    fn cppvtable_c_set(object: *mut c_void, value: i64);
    fn cppvtable_c_read(object: *mut c_void, input: *const i64) -> i64;
    fn cppvtable_c_write(object: *mut c_void, output: *mut i64);
}

#[test]
fn rust_calls_c_table() {
    // SAFETY: C allocates a matching object.
    let raw = unsafe { cppvtable_c_create(0x1_0000_002a) };
    {
        // SAFETY: The C object lives until the delete below, after the last borrow.
        let interface = unsafe { ICValue::from_raw(raw) }.expect("C allocation succeeded");
        assert_eq!(interface.get(), 0x1_0000_002a);
        interface.set(-7);
        assert_eq!(interface.get(), -7);
        // SAFETY: `raw` is the live C object.
        assert_eq!(unsafe { cppvtable_c_get(raw) }, -7);
        let input = 0x1_0000_0042_i64;
        // SAFETY: `input` is an initialized local.
        assert_eq!(unsafe { interface.read(&raw const input) }, input);
        let mut output = 0;
        // SAFETY: `output` is a writable local.
        unsafe { interface.write(&raw mut output) };
        assert_eq!(output, -7);
    }
    // SAFETY: The C allocation is deleted once, after its last borrow.
    unsafe { cppvtable_c_delete(raw) };
}

#[test]
fn c_calls_rust_table() {
    let owner = OwnedObject::new(Value {
        value: Cell::new(0x1_0000_002a),
    });
    let raw = owner.as_raw::<ICValue>();
    // SAFETY: C only borrows the interface pointer while the Rust owner is alive.
    unsafe {
        assert_eq!(cppvtable_c_get(raw), 0x1_0000_002a);
        cppvtable_c_set(raw, -7);
        assert_eq!(cppvtable_c_get(raw), -7);
        let input = 0x1_0000_0042_i64;
        assert_eq!(cppvtable_c_read(raw, &raw const input), input);
        let mut output = 0;
        cppvtable_c_write(raw, &raw mut output);
        assert_eq!(output, -7);
    }
    assert_eq!(owner.value.get(), -7);
}

#[test]
fn implementation_safety_distinguishes_plain_values_and_raw_storage() {
    let value = Value {
        value: Cell::new(42),
    };
    assert_eq!(value.get(), 42);
    value.set(7);
    assert_eq!(value.get(), 7);
    let input = 9;
    let mut output = 0;
    // SAFETY: Both arguments refer to aligned local storage valid through each call.
    unsafe {
        assert_eq!(value.read(&raw const input), 9);
        value.write(&raw mut output);
    }
    assert_eq!(output, 7);
}
