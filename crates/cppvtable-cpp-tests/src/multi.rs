//! Multiple inheritance C++ interop tests

use super::*;
use cppvtable::{Object, OwnedObject};

/// Test Rust can call a C++ multi-inheritance object through its primary interface.
#[test]
fn test_cpp_multi_inheritance_primary_interface() {
    unsafe {
        let cpp_duck = create_cpp_duck(10);
        let swimmer_ptr = cpp_duck_as_swimmer(cpp_duck);

        {
            // SAFETY: The C++ object stays alive through the borrowed interface call.
            let swimmer = IForeignSwimmer::from_raw_ref(&swimmer_ptr);
            assert_eq!(swimmer.swim_speed(), 10);
        }

        delete_cpp_duck(cpp_duck);
    }
}

/// Test Rust can call a C++ multi-inheritance object through its secondary interface.
#[test]
fn test_cpp_multi_inheritance_secondary_interface() {
    unsafe {
        let cpp_duck = create_cpp_duck(10);
        let flyer_ptr = cpp_duck_as_flyer(cpp_duck);

        {
            // SAFETY: The C++ object stays alive through the borrowed interface call.
            let flyer = IForeignFlyer::from_raw_ref(&flyer_ptr);
            assert_eq!(flyer.fly_speed(), 20);
        }

        delete_cpp_duck(cpp_duck);
    }
}

/// Test C++ can call a Rust multi-inheritance object through its primary interface.
#[test]
fn test_rust_multi_inheritance_cpp_calls_primary() {
    let rust_duck = OwnedObject::new(Duck::new(15));
    // SAFETY: The owner keeps its matching swimmer interface alive.
    assert_eq!(
        unsafe { cpp_call_swim_speed(rust_duck.as_raw::<ISwimmer>()) },
        15
    );
}

/// Test C++ can call a Rust multi-inheritance object through its secondary interface.
#[test]
fn test_rust_multi_inheritance_cpp_calls_secondary() {
    let rust_duck = OwnedObject::new(Duck::new(15));
    // SAFETY: The owner keeps its matching flyer interface alive.
    assert_eq!(
        unsafe { cpp_call_fly_speed(rust_duck.as_raw::<IFlyer>()) },
        30
    );
}

/// Test generated interface pointers preserve multiple-inheritance adjustment.
#[test]
fn test_multi_inheritance_layout() {
    let rust_duck = OwnedObject::new(Duck::new(10));
    let swimmer = rust_duck.as_raw::<ISwimmer>() as usize;
    let flyer = rust_duck.as_raw::<IFlyer>() as usize;

    assert_eq!(Object::<Duck>::slot_offset(0), 0);
    assert_eq!(
        Object::<Duck>::slot_offset(1),
        std::mem::size_of::<*const ()>()
    );
    assert_eq!(flyer - swimmer, Object::<Duck>::slot_offset(1));
}

/// Test C++ interface pointer offsets match the `static_cast` adjustment.
#[test]
fn test_cpp_interface_pointer_offsets() {
    let cpp_duck = create_cpp_duck(10);
    // SAFETY: The factory allocation is a live CppDuck through both casts and deletion.
    unsafe {
        let swimmer_ptr = cpp_duck_as_swimmer(cpp_duck);
        let flyer_ptr = cpp_duck_as_flyer(cpp_duck);

        // In the supported nonvirtual multiple-inheritance layout, the secondary interface is offset from the primary.
        let offset = (flyer_ptr as usize) - (swimmer_ptr as usize);

        assert_eq!(offset, std::mem::size_of::<*const ()>());

        delete_cpp_duck(cpp_duck);
    }
}
