//! Single inheritance C++ interop tests

use super::*;
use std::ffi::c_void;

/// Test that Rust can call C++ objects through our interface
#[test]
fn test_rust_calls_cpp_objects() {
    unsafe {
        let cpp_dog = create_cpp_dog("Max");
        let cpp_cat = create_cpp_cat(7);

        // Call through our Rust interface - this proves vtable layout matches
        let dog_ref = IAnimal::from_ptr_mut(cpp_dog);
        let cat_ref = IAnimal::from_ptr_mut(cpp_cat);

        assert_eq!(dog_ref.legs(), 4);
        assert_eq!(cat_ref.legs(), 4);

        delete_cpp_animal(cpp_dog);
        delete_cpp_animal(cpp_cat);
    }
}

/// Test that C++ can call Rust objects through vtable
#[test]
fn test_cpp_calls_rust_objects() {
    let rust_dog = Dog::new("Buddy");
    let rust_cat = Cat::new(9);

    let dog_ptr = &rust_dog as *const Dog as *mut c_void;
    let cat_ptr = &rust_cat as *const Cat as *mut c_void;

    // C++ calling through vtable - proves our vtable layout matches C++ expectations
    assert_eq!(cpp_call_rust_legs(dog_ptr), 4);
    assert_eq!(cpp_call_rust_legs(cat_ptr), 4);
}

/// Test vtable pointer is at offset 0
#[test]
fn test_vtable_at_offset_zero() {
    assert_eq!(std::mem::offset_of!(Dog, vtable_i_animal), 0);
    assert_eq!(std::mem::offset_of!(Cat, vtable_i_animal), 0);
}

/// Test struct sizes are correct
#[test]
fn test_struct_sizes() {
    // Dog: vtable ptr (8) + name[32] = 40 bytes
    assert_eq!(std::mem::size_of::<Dog>(), 40);

    // Cat: vtable ptr (8) + lives (4) + padding (4) = 16 bytes on x64
    #[cfg(target_pointer_width = "64")]
    assert_eq!(std::mem::size_of::<Cat>(), 16);
}

/// Test vtable size matches expected slot count
#[test]
fn test_vtable_size() {
    let ptr_size = std::mem::size_of::<*const ()>();
    assert_eq!(std::mem::size_of::<IAnimalVTable>(), 2 * ptr_size);
}

/// Test round-trip: create in C++, read in Rust, verify in C++
#[test]
fn test_cpp_rust_cpp_roundtrip() {
    unsafe {
        let cpp_dog = create_cpp_dog("Roundtrip");

        let dog_ref = IAnimal::from_ptr_mut(cpp_dog);
        let legs_via_rust = dog_ref.legs();
        let legs_via_cpp = cpp_call_legs(cpp_dog);

        assert_eq!(legs_via_rust, legs_via_cpp);
        assert_eq!(legs_via_rust, 4);

        delete_cpp_animal(cpp_dog);
    }
}
