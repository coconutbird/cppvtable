//! Bidirectional calls through a table compiled by a C compiler.

use cppvtable::{OwnedObject, implement, interface};
use std::cell::Cell;
use std::ffi::c_void;

#[interface(abi = c)]
unsafe trait ICValue {
    fn get(&self) -> i64;
    fn set(&self, value: i64);
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
}

unsafe extern "C" {
    fn cppvtable_c_create(value: i64) -> *mut c_void;
    fn cppvtable_c_delete(object: *mut c_void);
    fn cppvtable_c_get(object: *mut c_void) -> i64;
    fn cppvtable_c_set(object: *mut c_void, value: i64);
}

#[test]
fn rust_calls_c_table() {
    // SAFETY: C allocates the matching object and it lives through all calls.
    unsafe {
        let raw = cppvtable_c_create(0x1_0000_002a);
        assert!(!raw.is_null());
        let interface = ICValue::from_raw_ref(&raw);
        assert_eq!(interface.get(), 0x1_0000_002a);
        interface.set(-7);
        assert_eq!(interface.get(), -7);
        assert_eq!(cppvtable_c_get(raw), -7);
        cppvtable_c_delete(raw);
    }
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
    }
    assert_eq!(owner.get().value.get(), -7);
}
