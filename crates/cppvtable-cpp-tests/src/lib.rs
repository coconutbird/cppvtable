//! C++ interop tests for cppvtable
//!
//! This crate verifies that cppvtable's vtable layout matches MSVC's C++ vtable layout.
//! Requires MSVC to build and run.
//!
//! Run with: `cargo test -p cppvtable-cpp-tests`

#![recursion_limit = "512"]

use cpp::cpp;
use cppvtable::proc::{cpp_interface, implement};
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

    // ==========================================================================
    // Multiple inheritance interfaces and classes
    // ==========================================================================

    class ISwimmer {
    public:
        virtual int swim_speed() = 0;
        virtual void swim() = 0;
    };

    class IFlyer {
    public:
        virtual int fly_speed() = 0;
        virtual void fly() = 0;
    };

    // Duck implements both ISwimmer and IFlyer (multiple inheritance)
    class CppDuck : public ISwimmer, public IFlyer {
    public:
        int speed;

        CppDuck(int s) : speed(s) {}

        // ISwimmer
        int swim_speed() override { return speed; }
        void swim() override { printf("Duck swimming at %d\n", speed); }

        // IFlyer
        int fly_speed() override { return speed * 2; }
        void fly() override { printf("Duck flying at %d\n", speed * 2); }
    };
}}

// =============================================================================
// C++ helper functions
// Note: These cannot use #[cfg(test)] because cpp_build needs to see them
// =============================================================================

#[allow(dead_code)]
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

#[allow(dead_code)]
fn create_cpp_cat(lives: i32) -> *mut c_void {
    cpp!(unsafe [lives as "int"] -> *mut c_void as "void*" {
        return new CppCat(lives);
    })
}

#[allow(dead_code)]
fn cpp_call_legs(animal: *mut c_void) -> i32 {
    cpp!(unsafe [animal as "ICppAnimal*"] -> i32 as "int" {
        return animal->legs();
    })
}

#[allow(dead_code)]
fn delete_cpp_animal(animal: *mut c_void) {
    cpp!(unsafe [animal as "ICppAnimal*"] {
        delete animal;
    })
}

#[allow(dead_code)]
fn cpp_call_rust_legs(rust_animal: *mut c_void) -> i32 {
    cpp!(unsafe [rust_animal as "ICppAnimal*"] -> i32 as "int" {
        return rust_animal->legs();
    })
}

// Multiple inheritance helpers
#[allow(dead_code)]
fn create_cpp_duck(speed: i32) -> *mut c_void {
    cpp!(unsafe [speed as "int"] -> *mut c_void as "void*" {
        return new CppDuck(speed);
    })
}

#[allow(dead_code)]
fn delete_cpp_duck(duck: *mut c_void) {
    cpp!(unsafe [duck as "CppDuck*"] {
        delete duck;
    })
}

#[allow(dead_code)]
fn cpp_duck_as_swimmer(duck: *mut c_void) -> *mut c_void {
    cpp!(unsafe [duck as "CppDuck*"] -> *mut c_void as "void*" {
        return static_cast<ISwimmer*>(duck);
    })
}

#[allow(dead_code)]
fn cpp_duck_as_flyer(duck: *mut c_void) -> *mut c_void {
    cpp!(unsafe [duck as "CppDuck*"] -> *mut c_void as "void*" {
        return static_cast<IFlyer*>(duck);
    })
}

#[allow(dead_code)]
fn cpp_call_swim_speed(swimmer: *mut c_void) -> i32 {
    cpp!(unsafe [swimmer as "ISwimmer*"] -> i32 as "int" {
        return swimmer->swim_speed();
    })
}

#[allow(dead_code)]
fn cpp_call_fly_speed(flyer: *mut c_void) -> i32 {
    cpp!(unsafe [flyer as "IFlyer*"] -> i32 as "int" {
        return flyer->fly_speed();
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

// =============================================================================
// Multiple inheritance interfaces
// =============================================================================

#[cpp_interface]
pub trait ISwimmer {
    fn swim_speed(&self) -> i32;
    fn swim(&self);
}

#[cpp_interface]
pub trait IFlyer {
    fn fly_speed(&self) -> i32;
    fn fly(&self);
}

/// Rust Duck implementing both ISwimmer and IFlyer
#[repr(C)]
pub struct Duck {
    vtable_i_swimmer: *const ISwimmerVTable,
    vtable_i_flyer: *const IFlyerVTable,
    pub speed: i32,
}

#[implement(ISwimmer)]
impl Duck {
    fn swim_speed(&self) -> i32 {
        self.speed
    }
    fn swim(&self) {
        println!("Duck swimming at {}", self.speed);
    }
}

#[implement(IFlyer)]
impl Duck {
    fn fly_speed(&self) -> i32 {
        self.speed * 2
    }
    fn fly(&self) {
        println!("Duck flying at {}", self.speed * 2);
    }
}

impl Duck {
    pub fn new(speed: i32) -> Self {
        Duck {
            vtable_i_swimmer: Self::VTABLE_I_SWIMMER,
            vtable_i_flyer: Self::VTABLE_I_FLYER,
            speed,
        }
    }
}

// =============================================================================
// Single inheritance structs
// =============================================================================

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

// =============================================================================
// Multiple Inheritance Tests
// =============================================================================

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
