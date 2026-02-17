//! Tests for RTTI (Runtime Type Information) system

use cppvtable::rtti::{TypeInfo, InterfaceInfo, VTableWithRtti};
use std::ffi::c_void;

// Define interface IDs using static addresses (pointer-based for const-compatibility)
static IID_ISWIMMER: u8 = 0;
static IID_IFLYER: u8 = 0;
static IID_100: u8 = 0;
static IID_200: u8 = 0;

#[test]
fn test_type_info_creation() {
    let type_info = TypeInfo::new(0x12345678, "Duck", &[]);

    assert_eq!(type_info.type_id, 0x12345678);
    assert_eq!(type_info.type_name, "Duck");
}

#[test]
fn test_interface_info_size() {
    // InterfaceInfo should be 2 * pointer size
    let size = std::mem::size_of::<InterfaceInfo>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(size, 16);
    #[cfg(target_pointer_width = "32")]
    assert_eq!(size, 8);
}

#[test]
fn test_interface_info_const_creation() {
    // Test that InterfaceInfo can be created in const context
    const INFO: InterfaceInfo = InterfaceInfo::new(&IID_ISWIMMER as *const u8, 0);
    assert_eq!(INFO.offset, 0);
}

#[test]
fn test_type_info_implements() {
    // Leak to get 'static lifetime for test
    let interfaces: &'static [InterfaceInfo] = Box::leak(Box::new([
        InterfaceInfo::new(&IID_100 as *const u8, 0),
        InterfaceInfo::new(&IID_200 as *const u8, 8),
    ]));

    let type_info = TypeInfo::new(1, "TestType", interfaces);

    assert!(type_info.implements(&IID_100 as *const u8));
    assert!(type_info.implements(&IID_200 as *const u8));
    assert!(!type_info.implements(&IID_ISWIMMER as *const u8)); // Different static
}

#[test]
fn test_cast_to() {
    let interfaces: &'static [InterfaceInfo] = Box::leak(Box::new([
        InterfaceInfo::new(&IID_100 as *const u8, 0),
        InterfaceInfo::new(&IID_200 as *const u8, 8),
    ]));

    let type_info = TypeInfo::new(1, "TestType", interfaces);

    // Create a dummy object
    let dummy = [0u8; 32];
    let object_ptr = dummy.as_ptr() as *const c_void;

    unsafe {
        // Cast to interface at offset 0
        let ptr1 = type_info.cast_to(object_ptr, &IID_100 as *const u8);
        assert_eq!(ptr1, object_ptr);

        // Cast to interface at offset 8
        let ptr2 = type_info.cast_to(object_ptr, &IID_200 as *const u8);
        assert_eq!(ptr2, (object_ptr as *const u8).offset(8) as *const c_void);

        // Cast to non-existent interface returns null
        let ptr3 = type_info.cast_to(object_ptr, &IID_IFLYER as *const u8);
        assert!(ptr3.is_null());
    }
}

#[test]
fn test_vtable_with_rtti_layout() {
    // Test VTableWithRtti memory layout
    #[repr(C)]
    struct TestVTable {
        method1: extern "C" fn(),
        method2: extern "C" fn(),
    }

    // VTableWithRtti should have RTTI pointer first, then methods
    let size = std::mem::size_of::<VTableWithRtti<TestVTable>>();
    let ptr_size = std::mem::size_of::<*const TypeInfo>();
    let vtable_size = std::mem::size_of::<TestVTable>();

    assert_eq!(size, ptr_size + vtable_size);
}
