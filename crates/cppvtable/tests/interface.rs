//! Tests for #[cpp_interface] attribute

use cppvtable::proc::cpp_interface;
use std::ffi::c_void;

/// Basic interface with two methods
#[cpp_interface]
pub trait IBasic {
    fn get_value(&self) -> i32;
    fn set_value(&mut self, val: i32);
}

#[test]
fn test_vtable_struct_exists() {
    // VTable struct should be generated with correct name
    let _size = std::mem::size_of::<IBasicVTable>();
}

#[test]
fn test_vtable_has_correct_size() {
    // 2 methods = 2 function pointers = 16 bytes on x64
    let size = std::mem::size_of::<IBasicVTable>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(
        size, 16,
        "VTable should have 2 function pointers (16 bytes on x64)"
    );
    #[cfg(target_pointer_width = "32")]
    assert_eq!(
        size, 8,
        "VTable should have 2 function pointers (8 bytes on x86)"
    );
}

#[test]
fn test_interface_struct_exists() {
    // Interface wrapper struct should be generated
    let _size = std::mem::size_of::<IBasic>();
}

#[test]
fn test_interface_is_single_pointer() {
    // Interface should be just a vtable pointer
    let size = std::mem::size_of::<IBasic>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(
        size, 8,
        "Interface should be single pointer (8 bytes on x64)"
    );
    #[cfg(target_pointer_width = "32")]
    assert_eq!(
        size, 4,
        "Interface should be single pointer (4 bytes on x86)"
    );
}

#[test]
fn test_repr_c_layout() {
    // Both should have C-compatible layout
    assert_eq!(
        std::mem::align_of::<IBasicVTable>(),
        std::mem::align_of::<*const c_void>()
    );
    assert_eq!(
        std::mem::align_of::<IBasic>(),
        std::mem::align_of::<*const c_void>()
    );
}

/// Interface with various return types
#[cpp_interface]
pub trait IReturnTypes {
    fn returns_nothing(&self);
    fn returns_i32(&self) -> i32;
    fn returns_f32(&self) -> f32;
    fn returns_bool(&self) -> bool;
    fn returns_pointer(&self) -> *mut c_void;
}

#[test]
fn test_various_return_types() {
    // Should compile and have 5 function pointers
    let size = std::mem::size_of::<IReturnTypesVTable>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(size, 40, "5 methods = 40 bytes on x64");
    #[cfg(target_pointer_width = "32")]
    assert_eq!(size, 20, "5 methods = 20 bytes on x86");
}

/// Interface with parameters
#[cpp_interface]
pub trait IWithParams {
    fn no_params(&self);
    fn one_param(&self, a: i32);
    fn two_params(&self, a: i32, b: f32);
    fn three_params(&self, a: i32, b: f32, c: *mut c_void);
}

#[test]
fn test_methods_with_params() {
    // Should compile - parameter types don't affect vtable size
    let size = std::mem::size_of::<IWithParamsVTable>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(size, 32, "4 methods = 32 bytes on x64");
}

/// Interface with mutable self
#[cpp_interface]
pub trait IMutable {
    fn get(&self) -> i32;
    fn set(&mut self, val: i32);
    fn increment(&mut self);
}

#[test]
fn test_mutable_methods() {
    let size = std::mem::size_of::<IMutableVTable>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(size, 24, "3 methods = 24 bytes on x64");
}
