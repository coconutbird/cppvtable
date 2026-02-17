//! Tests for multiple inheritance with this-pointer adjustment

use cppvtable::proc::{cpp_interface, implement};
use std::ffi::c_void;

/// First interface
#[cpp_interface]
pub trait IFirst {
    fn first_method(&self) -> i32;
    fn first_value(&self) -> i32;
}

/// Second interface
#[cpp_interface]
pub trait ISecond {
    fn second_method(&self) -> i32;
    fn second_value(&self) -> i32;
}

/// Struct implementing both interfaces
#[repr(C)]
pub struct MultiImpl {
    vtable_i_first: *const IFirstVTable,
    vtable_i_second: *const ISecondVTable,
    value: i32,
}

#[implement(IFirst)]
impl MultiImpl {
    fn first_method(&self) -> i32 {
        100
    }
    fn first_value(&self) -> i32 {
        self.value
    }
}

#[implement(ISecond)]
impl MultiImpl {
    fn second_method(&self) -> i32 {
        200
    }
    fn second_value(&self) -> i32 {
        self.value * 2
    }
}

impl MultiImpl {
    pub fn new(value: i32) -> Self {
        MultiImpl {
            vtable_i_first: Self::VTABLE_I_FIRST,
            vtable_i_second: Self::VTABLE_I_SECOND,
            value,
        }
    }
}

#[test]
fn test_struct_layout() {
    // Two vtable pointers + i32 value
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(std::mem::offset_of!(MultiImpl, vtable_i_first), 0);
        assert_eq!(std::mem::offset_of!(MultiImpl, vtable_i_second), 8);
        assert_eq!(std::mem::offset_of!(MultiImpl, value), 16);
    }
}

#[test]
fn test_both_vtable_consts_exist() {
    let first = MultiImpl::VTABLE_I_FIRST;
    let second = MultiImpl::VTABLE_I_SECOND;
    assert!(!first.is_null());
    assert!(!second.is_null());
    assert_ne!(first as *const _ as usize, second as *const _ as usize);
}

#[test]
fn test_direct_calls() {
    let obj = MultiImpl::new(42);

    // IFirst methods
    assert_eq!(obj.first_method(), 100);
    assert_eq!(obj.first_value(), 42);

    // ISecond methods
    assert_eq!(obj.second_method(), 200);
    assert_eq!(obj.second_value(), 84);
}

#[test]
fn test_primary_interface_polymorphic() {
    let mut obj = MultiImpl::new(10);

    // Cast to IFirst (primary, offset 0 - no adjustment needed)
    let iface: &mut IFirst = unsafe { &mut *(&mut obj as *mut MultiImpl as *mut IFirst) };

    unsafe {
        assert_eq!(iface.first_method(), 100);
        assert_eq!(iface.first_value(), 10);
    }
}

#[test]
fn test_secondary_interface_polymorphic() {
    let mut obj = MultiImpl::new(10);

    // Cast to ISecond (secondary, offset 8 - requires this-adjustment)
    let iface: &mut ISecond = unsafe {
        let ptr = (&mut obj as *mut MultiImpl as *mut u8)
            .add(std::mem::offset_of!(MultiImpl, vtable_i_second));
        &mut *(ptr as *mut ISecond)
    };

    unsafe {
        // These calls go through vtable, wrapper adjusts this pointer back
        assert_eq!(iface.second_method(), 200);
        assert_eq!(iface.second_value(), 20); // 10 * 2
    }
}

#[test]
fn test_vtable_calls_through_secondary() {
    let obj = MultiImpl::new(7);

    unsafe {
        // Get pointer to secondary interface location
        let secondary_ptr = (&obj as *const MultiImpl as *const u8)
            .add(std::mem::offset_of!(MultiImpl, vtable_i_second));

        // Read vtable pointer
        let vtable = *(secondary_ptr as *const *const ISecondVTable);
        let vtable = &*vtable;

        // Call through vtable - this pointer is at secondary interface location
        // The wrapper should adjust it back to struct start
        let result = (vtable.second_method)(secondary_ptr as *mut c_void);
        assert_eq!(result, 200);

        let result = (vtable.second_value)(secondary_ptr as *mut c_void);
        assert_eq!(result, 14); // 7 * 2
    }
}
