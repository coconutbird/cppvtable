//! C++ interop tests for cppvtable
//!
//! This crate verifies that cppvtable's vtable layout matches MSVC's C++ vtable layout.
//! Requires MSVC to build and run.
//!
//! Run with: `cargo test -p cppvtable-cpp-tests`

#![recursion_limit = "512"]

use cpp::cpp;
use cppvtable::{ForwardRefCount, RefCounted, implement, interface};
use std::ffi::c_void;

#[cfg(test)]
mod multi;
#[cfg(test)]
mod single;

// C++ code compiled by MSVC

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

    // Multiple inheritance interfaces and classes

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

// C++ helper functions
// Note: These cannot use #[cfg(test)] because cpp_build needs to see them

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
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

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
fn create_cpp_cat(lives: i32) -> *mut c_void {
    cpp!(unsafe [lives as "int"] -> *mut c_void as "void*" {
        return new CppCat(lives);
    })
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
fn cpp_call_legs(animal: *mut c_void) -> i32 {
    cpp!(unsafe [animal as "ICppAnimal*"] -> i32 as "int" {
        return animal->legs();
    })
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
fn delete_cpp_animal(animal: *mut c_void) {
    cpp!(unsafe [animal as "ICppAnimal*"] {
        delete animal;
    });
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
fn cpp_call_rust_legs(rust_animal: *mut c_void) -> i32 {
    cpp!(unsafe [rust_animal as "ICppAnimal*"] -> i32 as "int" {
        return rust_animal->legs();
    })
}

// Multiple inheritance helpers
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
fn create_cpp_duck(speed: i32) -> *mut c_void {
    cpp!(unsafe [speed as "int"] -> *mut c_void as "void*" {
        return new CppDuck(speed);
    })
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
fn delete_cpp_duck(duck: *mut c_void) {
    cpp!(unsafe [duck as "CppDuck*"] {
        delete duck;
    });
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
fn cpp_duck_as_swimmer(duck: *mut c_void) -> *mut c_void {
    cpp!(unsafe [duck as "CppDuck*"] -> *mut c_void as "void*" {
        return static_cast<ISwimmer*>(duck);
    })
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
fn cpp_duck_as_flyer(duck: *mut c_void) -> *mut c_void {
    cpp!(unsafe [duck as "CppDuck*"] -> *mut c_void as "void*" {
        return static_cast<IFlyer*>(duck);
    })
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
fn cpp_call_swim_speed(swimmer: *mut c_void) -> i32 {
    cpp!(unsafe [swimmer as "ISwimmer*"] -> i32 as "int" {
        return swimmer->swim_speed();
    })
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "compiled for cpp_build and used in tests")
)]
fn cpp_call_fly_speed(flyer: *mut c_void) -> i32 {
    cpp!(unsafe [flyer as "IFlyer*"] -> i32 as "int" {
        return flyer->fly_speed();
    })
}

// Rust interfaces matching the C++ classes

#[interface(abi = cpp)]
unsafe trait IAnimal {
    fn speak(&self);
    fn legs(&self) -> i32;
}

#[interface(abi = cpp)]
unsafe trait ISwimmer {
    fn swim_speed(&self) -> i32;
    fn swim(&self);
}

#[interface(abi = cpp)]
unsafe trait IFlyer {
    fn fly_speed(&self) -> i32;
    fn fly(&self);
}

// Rust objects exposed through C++ interfaces

#[implement(ISwimmer, IFlyer)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used only by the C++ interop tests")
)]
struct Duck {
    speed: i32,
}

impl RefCounted for Duck {
    type Policy = ForwardRefCount;
}

impl ISwimmerImpl for Duck {
    fn swim_speed(&self) -> i32 {
        self.speed
    }

    fn swim(&self) {
        println!("Duck swimming at {}", self.speed);
    }
}

impl IFlyerImpl for Duck {
    fn fly_speed(&self) -> i32 {
        self.speed * 2
    }

    fn fly(&self) {
        println!("Duck flying at {}", self.speed * 2);
    }
}

impl Duck {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used only by the C++ interop tests")
    )]
    fn new(speed: i32) -> Self {
        Self { speed }
    }
}

#[implement(IAnimal)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used only by the C++ interop tests")
)]
struct Dog {
    name: [u8; 32],
}

impl RefCounted for Dog {
    type Policy = ForwardRefCount;
}

impl IAnimalImpl for Dog {
    fn speak(&self) {
        let name_len = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        let name = std::str::from_utf8(&self.name[..name_len]).unwrap_or("???");
        println!("{name} says: Woof!");
    }

    fn legs(&self) -> i32 {
        4
    }
}

impl Dog {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used only by the C++ interop tests")
    )]
    fn new(name: &str) -> Self {
        let mut dog = Self { name: [0_u8; 32] };
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        dog.name[..len].copy_from_slice(&bytes[..len]);
        dog
    }
}

#[implement(IAnimal)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used only by the C++ interop tests")
)]
struct Cat {
    lives: i32,
}

impl RefCounted for Cat {
    type Policy = ForwardRefCount;
}

impl IAnimalImpl for Cat {
    fn speak(&self) {
        println!("Cat with {} lives says: Meow!", self.lives);
    }

    fn legs(&self) -> i32 {
        4
    }
}

impl Cat {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used only by the C++ interop tests")
    )]
    fn new(lives: i32) -> Self {
        Self { lives }
    }
}
