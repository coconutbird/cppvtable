//! Single inheritance C++ interop tests

use super::*;
use cppvtable::{Object, OwnedObject};

/// Test that Rust can call C++ objects through our interface.
#[test]
fn test_rust_calls_cpp_objects() {
    unsafe {
        let cpp_dog = create_cpp_dog("Max");
        let cpp_cat = create_cpp_cat(7);

        {
            // SAFETY: Both C++ objects stay alive through the borrowed interface calls.
            let dog_ref = IForeignAnimal::from_raw_ref(&cpp_dog);
            let cat_ref = IForeignAnimal::from_raw_ref(&cpp_cat);

            assert_eq!(dog_ref.legs(), 4);
            assert_eq!(cat_ref.legs(), 4);
        }

        delete_cpp_dog(cpp_dog);
        delete_cpp_cat(cpp_cat);
    }
}

/// Test that C++ can call Rust objects through their generated vtables.
#[test]
fn test_cpp_calls_rust_objects() {
    let rust_dog = OwnedObject::new(Dog::new("Buddy"));
    let rust_cat = OwnedObject::new(Cat::new(9));

    // SAFETY: Each owning object keeps its matching animal interface alive.
    unsafe {
        assert_eq!(cpp_call_rust_legs(rust_dog.as_raw::<IAnimal>()), 4);
        assert_eq!(cpp_call_rust_legs(rust_cat.as_raw::<IAnimal>()), 4);
    }
}

/// Test that the primary interface vtable starts at offset zero.
#[test]
fn test_vtable_at_offset_zero() {
    assert_eq!(Object::<Dog>::slot_offset(0), 0);
    assert_eq!(Object::<Cat>::slot_offset(0), 0);
}

/// Test the generated vtable has the same slots as the C++ interface.
#[test]
fn test_vtable_size() {
    let ptr_size = std::mem::size_of::<*const ()>();
    assert_eq!(std::mem::size_of::<IForeignAnimalVtbl>(), 2 * ptr_size);
}

/// Test round-trip: create in C++, read in Rust, verify in C++.
#[test]
fn test_cpp_rust_cpp_roundtrip() {
    unsafe {
        let cpp_dog = create_cpp_dog("Roundtrip");

        {
            // SAFETY: The C++ object stays alive through both interface calls.
            let dog_ref = IForeignAnimal::from_raw_ref(&cpp_dog);
            let legs_via_rust = dog_ref.legs();
            let legs_via_cpp = cpp_call_legs(cpp_dog);

            assert_eq!(legs_via_rust, legs_via_cpp);
            assert_eq!(legs_via_rust, 4);
        }

        delete_cpp_dog(cpp_dog);
    }
}
