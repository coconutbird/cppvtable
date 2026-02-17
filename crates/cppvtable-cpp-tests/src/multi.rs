//! Multiple inheritance C++ interop tests

use super::*;
use std::ffi::c_void;

/// Test Rust can call C++ multi-inheritance object through primary interface
#[test]
fn test_cpp_multi_inheritance_primary_interface() {
    unsafe {
        let cpp_duck = create_cpp_duck(10);
        let swimmer_ptr = cpp_duck_as_swimmer(cpp_duck);

        // Call through Rust ISwimmer interface
        let swimmer = ISwimmer::from_ptr_mut(swimmer_ptr);
        assert_eq!(swimmer.swim_speed(), 10);

        delete_cpp_duck(cpp_duck);
    }
}

/// Test Rust can call C++ multi-inheritance object through secondary interface
#[test]
fn test_cpp_multi_inheritance_secondary_interface() {
    unsafe {
        let cpp_duck = create_cpp_duck(10);
        let flyer_ptr = cpp_duck_as_flyer(cpp_duck);

        // Call through Rust IFlyer interface
        let flyer = IFlyer::from_ptr_mut(flyer_ptr);
        assert_eq!(flyer.fly_speed(), 20); // speed * 2

        delete_cpp_duck(cpp_duck);
    }
}

/// Test C++ can call Rust multi-inheritance object through primary interface
#[test]
fn test_rust_multi_inheritance_cpp_calls_primary() {
    let rust_duck = Duck::new(15);
    let duck_ptr = &rust_duck as *const Duck as *mut c_void;

    // C++ calling through ISwimmer vtable (primary, offset 0)
    assert_eq!(cpp_call_swim_speed(duck_ptr), 15);
}

/// Test C++ can call Rust multi-inheritance object through secondary interface
#[test]
fn test_rust_multi_inheritance_cpp_calls_secondary() {
    let rust_duck = Duck::new(15);

    // Get pointer to secondary interface (IFlyer at offset 8)
    let flyer_ptr = unsafe {
        let ptr = &rust_duck as *const Duck as *const u8;
        ptr.add(std::mem::offset_of!(Duck, vtable_i_flyer)) as *mut c_void
    };

    // C++ calling through IFlyer vtable (secondary)
    assert_eq!(cpp_call_fly_speed(flyer_ptr), 30); // 15 * 2
}

/// Test multi-inheritance struct layout matches C++ expectations
#[test]
fn test_multi_inheritance_layout() {
    // Duck: vtable_swimmer (8) + vtable_flyer (8) + speed (4) + padding (4) = 24 bytes
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(std::mem::size_of::<Duck>(), 24);
        assert_eq!(std::mem::offset_of!(Duck, vtable_i_swimmer), 0);
        assert_eq!(std::mem::offset_of!(Duck, vtable_i_flyer), 8);
        assert_eq!(std::mem::offset_of!(Duck, speed), 16);
    }
}

/// Test pointer difference between interfaces matches C++ static_cast
#[test]
fn test_cpp_interface_pointer_offsets() {
    let cpp_duck = create_cpp_duck(10);
    let swimmer_ptr = cpp_duck_as_swimmer(cpp_duck);
    let flyer_ptr = cpp_duck_as_flyer(cpp_duck);

    // In MSVC multiple inheritance, secondary interface is at offset from primary
    let offset = (flyer_ptr as usize) - (swimmer_ptr as usize);

    // IFlyer should be at offset 8 (one vtable pointer) from ISwimmer
    #[cfg(target_pointer_width = "64")]
    assert_eq!(offset, 8);

    delete_cpp_duck(cpp_duck);
}
