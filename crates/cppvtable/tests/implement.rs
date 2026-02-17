//! Tests for #[cppvtable_impl] attribute

use cppvtable::proc::{cppvtable, cppvtable_impl};
use std::ffi::c_void;

/// Simple counter interface
#[cppvtable]
pub trait ICounter {
    fn get(&self) -> i32;
    fn increment(&mut self);
    fn add(&mut self, n: i32);
}

/// Struct implementing ICounter
#[repr(C)]
pub struct Counter {
    vtable_i_counter: *const ICounterVTable,
    value: i32,
}

#[cppvtable_impl(ICounter)]
impl Counter {
    fn get(&self) -> i32 {
        self.value
    }

    fn increment(&mut self) {
        self.value += 1;
    }

    fn add(&mut self, n: i32) {
        self.value += n;
    }
}

impl Counter {
    pub fn new(initial: i32) -> Self {
        Counter {
            vtable_i_counter: Self::VTABLE_I_COUNTER,
            value: initial,
        }
    }
}

#[test]
fn test_vtable_const_exists() {
    // VTABLE_I_COUNTER should be generated
    let ptr = Counter::VTABLE_I_COUNTER;
    assert!(!ptr.is_null());
}

#[test]
fn test_direct_method_calls() {
    let mut counter = Counter::new(10);
    assert_eq!(counter.get(), 10);

    counter.increment();
    assert_eq!(counter.get(), 11);

    counter.add(5);
    assert_eq!(counter.get(), 16);
}

#[test]
fn test_vtable_calls() {
    let mut counter = Counter::new(0);

    unsafe {
        // Get vtable
        let vtable = &*counter.vtable_i_counter;

        // Call through vtable
        let this = &mut counter as *mut Counter as *mut c_void;

        let val = (vtable.get)(this);
        assert_eq!(val, 0);

        (vtable.increment)(this);
        let val = (vtable.get)(this);
        assert_eq!(val, 1);

        (vtable.add)(this, 10);
        let val = (vtable.get)(this);
        assert_eq!(val, 11);
    }
}

#[test]
fn test_polymorphic_calls() {
    let mut counter = Counter::new(5);

    // Cast to interface
    let iface: &mut ICounter = unsafe { &mut *(&mut counter as *mut Counter as *mut ICounter) };

    unsafe {
        assert_eq!(iface.get(), 5);
        iface.increment();
        assert_eq!(iface.get(), 6);
        iface.add(4);
        assert_eq!(iface.get(), 10);
    }
}

#[test]
fn test_struct_layout() {
    // vtable should be at offset 0
    assert_eq!(std::mem::offset_of!(Counter, vtable_i_counter), 0);

    // value should be at offset 8 (pointer size on x64)
    #[cfg(target_pointer_width = "64")]
    assert_eq!(std::mem::offset_of!(Counter, value), 8);
}

/// Test with a more complex struct
#[cppvtable]
pub trait INamed {
    fn name_ptr(&self) -> *const u8;
    fn name_len(&self) -> usize;
}

#[repr(C)]
pub struct NamedThing {
    vtable_i_named: *const INamedVTable,
    name: [u8; 32],
    len: usize,
}

#[cppvtable_impl(INamed)]
impl NamedThing {
    fn name_ptr(&self) -> *const u8 {
        self.name.as_ptr()
    }

    fn name_len(&self) -> usize {
        self.len
    }
}

impl NamedThing {
    pub fn new(s: &str) -> Self {
        let mut thing = NamedThing {
            vtable_i_named: Self::VTABLE_I_NAMED,
            name: [0u8; 32],
            len: 0,
        };
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        thing.name[..len].copy_from_slice(&bytes[..len]);
        thing.len = len;
        thing
    }
}

#[test]
fn test_complex_struct() {
    let thing = NamedThing::new("Hello");

    unsafe {
        let vtable = &*thing.vtable_i_named;
        let this = &thing as *const NamedThing as *mut c_void;

        let ptr = (vtable.name_ptr)(this);
        let len = (vtable.name_len)(this);

        let slice = std::slice::from_raw_parts(ptr, len);
        assert_eq!(slice, b"Hello");
    }
}

// ============== RTTI Tests ==============

#[test]
fn test_interface_id_exists() {
    // Each interface should have an interface_id() method
    let id1 = ICounter::interface_id();
    let id2 = INamed::interface_id();

    // IDs should be different (different statics)
    assert_ne!(id1, id2);
}

#[test]
fn test_interface_id_ptr_is_const() {
    // interface_id_ptr() should be usable in const context
    const PTR: *const u8 = ICounter::interface_id_ptr();
    assert!(!PTR.is_null());
}

#[test]
fn test_interface_info_const_exists() {
    // INTERFACE_INFO_I_COUNTER should be generated
    let info = Counter::INTERFACE_INFO_I_COUNTER;

    // interface_id should match ICounter's ID
    assert_eq!(info.interface_id, ICounter::interface_id_ptr());

    // offset should be 0 (vtable at start of struct)
    assert_eq!(info.offset, 0);
}

#[test]
fn test_interface_info_offset() {
    // INTERFACE_INFO_I_NAMED should have correct offset
    let info = NamedThing::INTERFACE_INFO_I_NAMED;

    // offset should match actual struct layout
    assert_eq!(
        info.offset as usize,
        std::mem::offset_of!(NamedThing, vtable_i_named)
    );
}
