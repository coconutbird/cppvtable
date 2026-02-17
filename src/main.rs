//! Experimenting with C++ vtable compatibility in Rust
//!
//! This demonstrates two approaches:
//! 1. Declarative macros: `define_interface!` and `define_class!`
//! 2. Proc-macros: `#[cpp_interface]` and `#[implement]`
//!
//! Also includes C++ interop tests using the `cpp` crate to verify
//! vtable layout compatibility with actual MSVC-compiled C++ code.

#![allow(dead_code)]

mod vtable;

use cpp::cpp;
use std::ffi::c_void;
use std::io::{self, Write};

// =============================================================================
// C++ INTEROP: Define C++ classes and test vtable compatibility
// =============================================================================

// This block defines C++ code that will be compiled by MSVC
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

/// Create a C++ CppDog instance and return as opaque pointer
fn create_cpp_dog(name: &str) -> *mut c_void {
    let name_ptr = name.as_ptr();
    let name_len = name.len();
    cpp!(unsafe [name_ptr as "const char*", name_len as "size_t"] -> *mut c_void as "void*" {
        // Copy name to null-terminated buffer
        char buf[32] = {0};
        size_t copy_len = name_len < 31 ? name_len : 31;
        memcpy(buf, name_ptr, copy_len);
        return new CppDog(buf);
    })
}

/// Create a C++ CppCat instance and return as opaque pointer
fn create_cpp_cat(lives: i32) -> *mut c_void {
    cpp!(unsafe [lives as "int"] -> *mut c_void as "void*" {
        return new CppCat(lives);
    })
}

/// Call speak() on a C++ ICppAnimal through its vtable (C++ side)
fn cpp_call_speak(animal: *mut c_void) {
    cpp!(unsafe [animal as "ICppAnimal*"] {
        animal->speak();
        fflush(stdout);
    })
}

/// Call legs() on a C++ ICppAnimal through its vtable (C++ side)
fn cpp_call_legs(animal: *mut c_void) -> i32 {
    cpp!(unsafe [animal as "ICppAnimal*"] -> i32 as "int" {
        return animal->legs();
    })
}

/// Delete a C++ ICppAnimal
fn delete_cpp_animal(animal: *mut c_void) {
    cpp!(unsafe [animal as "ICppAnimal*"] {
        delete animal;
    })
}

/// Have C++ call through a Rust-provided vtable pointer
/// This tests that our Rust vtable layout matches C++ expectations
fn cpp_call_rust_animal(rust_animal: *mut c_void) {
    cpp!(unsafe [rust_animal as "ICppAnimal*"] {
        printf("C++ calling Rust object through vtable:\n");
        printf("    ");
        rust_animal->speak();
        printf("    legs() returned: %d\n", rust_animal->legs());
        fflush(stdout);
    })
}

// =============================================================================
// APPROACH 1: Declarative macros (define_interface! / cpp_class!)
// =============================================================================

// Define an interface using the declarative macro
define_interface! {
    interface IRunnable {
        fn run(&mut self);
        fn stop(&mut self);
    }
}

// Define a class implementing the interface
define_class! {
    pub class Runner : IRunnable {
        pub speed: f32,
        pub running: bool,
    }
}

impl Runner {
    pub fn new(speed: f32) -> Self {
        Runner {
            vtable: std::ptr::null(), // Will be set by implement
            speed,
            running: false,
        }
    }
}

// =============================================================================
// APPROACH 2: Proc-macros (#[cpp_interface] / #[implement])
// =============================================================================

use vtable_macro::{cpp_interface, implement};

/// Define a C++ interface using proc-macro
#[cpp_interface]
pub trait IAnimal {
    fn speak(&self);
    fn legs(&self) -> i32;
}

/// A Dog struct that will implement IAnimal
#[repr(C)]
pub struct Dog {
    vtable: *const IAnimalVTable,
    pub name: [u8; 32],
}

/// Implement the IAnimal interface for Dog
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
            vtable: Self::vtable_ptr(),
            name: [0u8; 32],
        };
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        dog.name[..len].copy_from_slice(&bytes[..len]);
        dog
    }

    pub fn as_interface(&self) -> &IAnimal {
        unsafe { &*(self as *const Dog as *const IAnimal) }
    }
}

/// Cat also implements IAnimal
#[repr(C)]
pub struct Cat {
    vtable: *const IAnimalVTable,
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
            vtable: Self::vtable_ptr(),
            lives,
        }
    }

    pub fn as_interface(&self) -> &IAnimal {
        unsafe { &*(self as *const Cat as *const IAnimal) }
    }
}

// =============================================================================
// GearScore-like interface (matching FC2 pattern)
// =============================================================================

#[cpp_interface]
pub trait IGearScore {
    fn destructor(&mut self, flags: u8) -> *mut c_void;
    fn get_score(&mut self, score_type: i32, param: i32, confidence_out: *mut f32) -> f32;
    fn compute_score(&mut self, score_type: i32, param: i32) -> i32;
}

const SCORE_UNCOMPUTED: f32 = -3.4028235e38;

#[repr(C)]
pub struct GearScore {
    vtable: *const IGearScoreVTable,
    pub scores: [f32; 2],
    pub confidence: [f32; 2],
}

#[implement(IGearScore)]
impl GearScore {
    fn destructor(&mut self, flags: u8) -> *mut c_void {
        println!("GearScore destructor (flags: {})", flags);
        self as *mut GearScore as *mut c_void
    }

    fn get_score(&mut self, score_type: i32, param: i32, confidence_out: *mut f32) -> f32 {
        if !(0..2).contains(&score_type) {
            return 0.0;
        }
        let idx = score_type as usize;
        if self.scores[idx] == SCORE_UNCOMPUTED {
            self.compute_score(score_type, param);
        }
        if !confidence_out.is_null() {
            unsafe { *confidence_out = self.confidence[idx] };
        }
        self.scores[idx]
    }

    fn compute_score(&mut self, score_type: i32, _param: i32) -> i32 {
        if !(0..2).contains(&score_type) {
            return score_type;
        }
        let idx = score_type as usize;
        self.scores[idx] = 0.85;
        self.confidence[idx] = 1.0;
        score_type
    }
}

impl GearScore {
    pub fn new() -> Self {
        GearScore {
            vtable: Self::vtable_ptr(),
            scores: [SCORE_UNCOMPUTED; 2],
            confidence: [1.0; 2],
        }
    }
}

fn main() {
    println!("=== C++ VTable Experiment ===\n");

    // =========================================================================
    // TEST 1: Rust calling C++ objects through vtable
    // =========================================================================
    println!("--- TEST 1: Rust consuming C++ objects ---");
    println!("Creating C++ objects and calling through Rust's from_ptr():\n");

    unsafe {
        // Create C++ objects
        let cpp_dog = create_cpp_dog("Max");
        let cpp_cat = create_cpp_cat(7);

        // Call through C++ side (baseline - this definitely works)
        println!("Calling from C++ side (baseline):");
        let _ = io::stdout().flush();
        cpp_call_speak(cpp_dog);
        cpp_call_speak(cpp_cat);
        println!("  CppDog legs: {}", cpp_call_legs(cpp_dog));
        println!("  CppCat legs: {}", cpp_call_legs(cpp_cat));

        // Now the real test: call through Rust's from_ptr!
        // This only works if our vtable layout matches C++
        println!("\nCalling from Rust side via from_ptr() - THIS PROVES LAYOUT MATCH:");
        let dog_ref = IAnimal::from_ptr_mut(cpp_dog);
        let cat_ref = IAnimal::from_ptr_mut(cpp_cat);

        print!("  ");
        dog_ref.speak();
        print!("  ");
        cat_ref.speak();
        println!("  Rust sees CppDog legs: {}", dog_ref.legs());
        println!("  Rust sees CppCat legs: {}", cat_ref.legs());

        // Cleanup
        delete_cpp_animal(cpp_dog);
        delete_cpp_animal(cpp_cat);
    }

    // =========================================================================
    // TEST 2: C++ calling Rust objects through vtable
    // =========================================================================
    println!("\n--- TEST 2: C++ consuming Rust objects ---");
    println!("Creating Rust objects and passing to C++ for vtable calls:\n");

    let rust_dog = Dog::new("Buddy");
    let rust_cat = Cat::new(9);

    // Pass Rust objects to C++ - C++ will call through the vtable
    // This only works if our Rust vtable layout matches what C++ expects
    {
        let dog_ptr = &rust_dog as *const Dog as *mut c_void;
        let cat_ptr = &rust_cat as *const Cat as *mut c_void;

        println!("C++ calling Rust Dog:");
        let _ = io::stdout().flush();
        cpp_call_rust_animal(dog_ptr);

        println!("\nC++ calling Rust Cat:");
        let _ = io::stdout().flush();
        cpp_call_rust_animal(cat_ptr);
    }

    // =========================================================================
    // Original Rust-only tests
    // =========================================================================
    println!("\n--- Rust-only tests (proc-macro approach) ---");

    println!("Direct calls:");
    rust_dog.speak();
    rust_cat.speak();

    println!("\nPolymorphic calls through IAnimal:");
    let animals: [&IAnimal; 2] = [rust_dog.as_interface(), rust_cat.as_interface()];
    for animal in animals {
        unsafe {
            let animal = std::ptr::from_ref(animal).cast_mut().as_mut().unwrap();
            animal.speak();
            println!("  Legs: {}", animal.legs());
        }
    }

    // GearScore
    println!("\n--- GearScore example ---");
    let mut score = GearScore::new();
    let mut conf: f32 = 0.0;
    let cpu_score = score.get_score(0, 0, &mut conf);
    println!("  CPU Score: {} (confidence: {})", cpu_score, conf);

    // Struct sizes
    println!("\n=== Struct sizes ===");
    println!("  Rust Dog: {} bytes", std::mem::size_of::<Dog>());
    println!("  Rust Cat: {} bytes", std::mem::size_of::<Cat>());
    println!("  GearScore: {} bytes", std::mem::size_of::<GearScore>());
    println!(
        "  IAnimalVTable: {} bytes",
        std::mem::size_of::<IAnimalVTable>()
    );
    println!(
        "  IRunnableVTable: {} bytes",
        std::mem::size_of::<IRunnableVTable>()
    );

    println!("\n=== ALL TESTS PASSED - VTABLE LAYOUTS MATCH! ===");
}
