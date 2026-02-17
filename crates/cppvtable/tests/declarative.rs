//! Tests for declarative macros (define_interface!, define_class!)

use cppvtable::proc::cppvtable_impl;
use cppvtable::{define_class, define_interface};

// =============================================================================
// Test define_interface! macro
// =============================================================================

define_interface! {
    interface ISimple {
        fn get_value(&self) -> i32;
        fn set_value(&mut self, val: i32);
    }
}

#[test]
fn test_define_interface_creates_vtable() {
    let size = std::mem::size_of::<ISimpleVTable>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(size, 16, "2 methods = 16 bytes on x64");
}

#[test]
fn test_define_interface_creates_interface_struct() {
    let size = std::mem::size_of::<ISimple>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(size, 8, "Interface is single pointer");
}

// Test interface with explicit slots
define_interface! {
    interface IWithSlots {
        fn at_zero(&self) -> i32;
        [3] fn at_three(&self) -> i32;
        fn at_four(&self) -> i32;
    }
}

#[test]
fn test_define_interface_with_slots() {
    // Slots 0, 1, 2, 3, 4 = 5 pointers
    let size = std::mem::size_of::<IWithSlotsVTable>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(size, 40, "5 slots = 40 bytes on x64");
}

// =============================================================================
// Test define_class! macro - single inheritance
// =============================================================================

define_class! {
    pub class SimpleClass : ISimple {
        pub value: i32,
    }
}

#[cppvtable_impl(ISimple)]
impl SimpleClass {
    fn get_value(&self) -> i32 {
        self.value
    }
    fn set_value(&mut self, val: i32) {
        self.value = val;
    }
}

impl SimpleClass {
    pub fn new(val: i32) -> Self {
        SimpleClass {
            vtable_i_simple: Self::VTABLE_I_SIMPLE,
            value: val,
        }
    }
}

#[test]
fn test_define_class_layout() {
    // vtable at offset 0, then value
    assert_eq!(std::mem::offset_of!(SimpleClass, vtable_i_simple), 0);
    #[cfg(target_pointer_width = "64")]
    assert_eq!(std::mem::offset_of!(SimpleClass, value), 8);
}

#[test]
fn test_define_class_methods() {
    let mut obj = SimpleClass::new(10);
    assert_eq!(obj.get_value(), 10);
    obj.set_value(20);
    assert_eq!(obj.get_value(), 20);
}

#[test]
fn test_define_class_cast_helper() {
    let mut obj = SimpleClass::new(42);
    let iface = obj.as_i_simple_mut();

    unsafe {
        assert_eq!(iface.get_value(), 42);
    }
}

// =============================================================================
// Test define_class! macro - multiple inheritance
// =============================================================================

define_interface! {
    interface IFirst {
        fn first(&self) -> i32;
    }

    interface ISecond {
        fn second(&self) -> i32;
    }
}

define_class! {
    pub class MultiClass : IFirst, ISecond {
        pub data: i32,
    }
}

#[cppvtable_impl(IFirst)]
impl MultiClass {
    fn first(&self) -> i32 {
        self.data
    }
}

#[cppvtable_impl(ISecond)]
impl MultiClass {
    fn second(&self) -> i32 {
        self.data * 2
    }
}

impl MultiClass {
    pub fn new(data: i32) -> Self {
        MultiClass {
            vtable_i_first: Self::VTABLE_I_FIRST,
            vtable_i_second: Self::VTABLE_I_SECOND,
            data,
        }
    }
}

#[test]
fn test_multi_class_layout() {
    assert_eq!(std::mem::offset_of!(MultiClass, vtable_i_first), 0);
    #[cfg(target_pointer_width = "64")]
    assert_eq!(std::mem::offset_of!(MultiClass, vtable_i_second), 8);
}

#[test]
fn test_multi_class_cast_helpers() {
    let mut obj = MultiClass::new(5);

    unsafe {
        let first = obj.as_i_first_mut();
        assert_eq!(first.first(), 5);
    }

    unsafe {
        let second = obj.as_i_second_mut();
        assert_eq!(second.second(), 10);
    }
}
