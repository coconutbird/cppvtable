//! C++ interop tests for cppvtable
//!
//! This crate verifies that cppvtable's vtable layout matches MSVC's C++ vtable layout.
//! Requires MSVC to build and run.
//!
//! Run with: `cargo test -p cppvtable-cpp-tests`

use cpp::cpp;
use cppvtable::proc::{cpp_interface, implement};
#[cfg(test)]
use std::ffi::c_void;

// =============================================================================
// C++ code compiled by MSVC
// =============================================================================

cpp! {{
    #include <cstdio>
    #include <cstring>

    // Pure virtual interface - should match our Rust IAnimal layout
    class ICppAnimal {
    public:
        virtual void speak() = 0;
        virtual int legs() = 0;
    };

    // Concrete C++ implementation
    class CppDog : public ICppAnimal {
    public:
        char name[32];

        CppDog(const char* n) {
            strncpy_s(name, sizeof(name), n, _TRUNCATE);
        }

        void speak() override {
            printf("CppDog '%s' says: Woof from C++!\n", name);
        }

        int legs() override {
            return 4;
        }
    };

    class CppCat : public ICppAnimal {
    public:
        int lives;

        CppCat(int l) : lives(l) {}

        void speak() override {
            printf("CppCat with %d lives says: Meow from C++!\n", lives);
        }

        int legs() override {
            return 4;
        }
    };
}}

// =============================================================================
// C++ helper functions (only used in tests)
// =============================================================================

#[cfg(test)]
fn create_cpp_dog(name: &str) -> *mut c_void {
    let name_ptr = name.as_ptr();
    let name_len = name.len();
    cpp!(unsafe [name_ptr as "const char*", name_len as "size_t"] -> *mut c_void as "void*" {
        char buf[32] = {0};
        size_t copy_len = name_len < 31 ? name_len : 31;
        memcpy(buf, name_ptr, copy_len);
        return new CppDog(buf);
    })
}

#[cfg(test)]
fn create_cpp_cat(lives: i32) -> *mut c_void {
    cpp!(unsafe [lives as "int"] -> *mut c_void as "void*" {
        return new CppCat(lives);
    })
}

#[cfg(test)]
fn cpp_call_legs(animal: *mut c_void) -> i32 {
    cpp!(unsafe [animal as "ICppAnimal*"] -> i32 as "int" {
        return animal->legs();
    })
}

#[cfg(test)]
fn delete_cpp_animal(animal: *mut c_void) {
    cpp!(unsafe [animal as "ICppAnimal*"] {
        delete animal;
    })
}

#[cfg(test)]
fn cpp_call_rust_legs(rust_animal: *mut c_void) -> i32 {
    cpp!(unsafe [rust_animal as "ICppAnimal*"] -> i32 as "int" {
        return rust_animal->legs();
    })
}

// =============================================================================
// Rust interface matching C++ ICppAnimal
// =============================================================================

#[cpp_interface]
pub trait IAnimal {
    fn speak(&self);
    fn legs(&self) -> i32;
}

#[repr(C)]
pub struct Dog {
    vtable_i_animal: *const IAnimalVTable,
    pub name: [u8; 32],
}

#[implement(IAnimal)]
impl Dog {
    fn speak(&self) {
        let name_len = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        let name = std::str::from_utf8(&self.name[..name_len]).unwrap_or("???");
        println!("{} says: Woof!", name);
    }

    fn legs(&self) -> i32 {
        4
    }
}

impl Dog {
    pub fn new(name: &str) -> Self {
        let mut dog = Dog {
            vtable_i_animal: Self::VTABLE_I_ANIMAL,
            name: [0u8; 32],
        };
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        dog.name[..len].copy_from_slice(&bytes[..len]);
        dog
    }
}

#[repr(C)]
pub struct Cat {
    vtable_i_animal: *const IAnimalVTable,
    pub lives: i32,
}

#[implement(IAnimal)]
impl Cat {
    fn speak(&self) {
        println!("Cat with {} lives says: Meow!", self.lives);
    }

    fn legs(&self) -> i32 {
        4
    }
}

impl Cat {
    pub fn new(lives: i32) -> Self {
        Cat {
            vtable_i_animal: Self::VTABLE_I_ANIMAL,
            lives,
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

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
