//! Experimenting with C++ vtable compatibility in Rust
//!
//! This demonstrates two approaches:
//! 1. Declarative macros: `define_interface!` and `define_class!`
//! 2. Proc-macros: `#[cpp_interface]` and `#[implement]`
//!
//! Also includes C++ interop tests using the `cpp` crate to verify
//! vtable layout compatibility with actual MSVC-compiled C++ code.

#![allow(dead_code)]

// Use the cppvtable crate from crates/cppvtable
// Declarative macros are #[macro_export] so they're at crate root
use cppvtable::{define_class, define_interface};

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
            vtable_i_runnable: std::ptr::null(), // Will be set by implement
            speed,
            running: false,
        }
    }
}

// =============================================================================
// APPROACH 2: Proc-macros (#[cpp_interface] / #[implement])
// =============================================================================

use cppvtable::proc::{cpp_interface, implement};

/// Define a C++ interface using proc-macro
#[cpp_interface]
pub trait IAnimal {
    fn speak(&self);
    fn legs(&self) -> i32;
}

/// A Dog struct that will implement IAnimal
#[repr(C)]
pub struct Dog {
    vtable_i_animal: *const IAnimalVTable,
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
            vtable_i_animal: &__DOG_IANIMAL_VTABLE,
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
            vtable_i_animal: &__CAT_IANIMAL_VTABLE,
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
    vtable_i_gear_score: *const IGearScoreVTable,
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
            vtable_i_gear_score: &__GEARSCORE_IGEARSCORE_VTABLE,
            scores: [SCORE_UNCOMPUTED; 2],
            confidence: [1.0; 2],
        }
    }
}

// =============================================================================
// TEST: #[slot(N)] attribute for explicit vtable slot indices
// =============================================================================

/// Interface with explicit slot indices - slots 0, 1, 5, 6
#[cpp_interface]
pub trait ISlotTest {
    fn method_at_0(&self) -> i32; // slot 0
    fn method_at_1(&self) -> i32; // slot 1
    #[slot(5)]
    fn method_at_5(&self) -> i32; // slot 5 (slots 2-4 are reserved)
    fn method_at_6(&self) -> i32; // slot 6
}

#[repr(C)]
pub struct SlotTester {
    vtable_i_slot_test: *const ISlotTestVTable,
}

#[implement(ISlotTest)]
impl SlotTester {
    fn method_at_0(&self) -> i32 {
        0
    }
    fn method_at_1(&self) -> i32 {
        1
    }
    #[slot(5)]
    fn method_at_5(&self) -> i32 {
        5
    }
    fn method_at_6(&self) -> i32 {
        6
    }
}

impl SlotTester {
    pub fn new() -> Self {
        SlotTester {
            vtable_i_slot_test: &__SLOTTESTER_ISLOTTEST_VTABLE,
        }
    }
}

// =============================================================================
// TEST: Multiple inheritance
// =============================================================================

/// Interface for things that can swim
#[cpp_interface]
pub trait ISwimmer {
    fn swim(&self);
    fn swim_speed(&self) -> f32;
}

/// Interface for things that can fly
#[cpp_interface]
pub trait IFlyer {
    fn fly(&self);
    fn fly_altitude(&self) -> f32;
}

/// A duck can both swim and fly - multiple inheritance!
#[repr(C)]
pub struct Duck {
    // Multiple vtable pointers - one per interface
    vtable_i_swimmer: *const ISwimmerVTable,
    vtable_i_flyer: *const IFlyerVTable,
    pub name: [u8; 16],
}

// Implement ISwimmer for Duck
#[implement(ISwimmer)]
impl Duck {
    fn swim(&self) {
        let name = std::str::from_utf8(&self.name)
            .unwrap_or("?")
            .trim_end_matches('\0');
        println!("{} is swimming!", name);
    }
    fn swim_speed(&self) -> f32 {
        2.5
    }
}

// Implement IFlyer for Duck (separate impl block)
#[implement(IFlyer)]
impl Duck {
    fn fly(&self) {
        let name = std::str::from_utf8(&self.name)
            .unwrap_or("?")
            .trim_end_matches('\0');
        println!("{} is flying!", name);
    }
    fn fly_altitude(&self) -> f32 {
        100.0
    }
}

impl Duck {
    pub fn new(name: &str) -> Self {
        let mut duck = Duck {
            vtable_i_swimmer: Duck::vtable_ptr_i_swimmer(),
            vtable_i_flyer: Duck::vtable_ptr_i_flyer(),
            name: [0u8; 16],
        };
        let bytes = name.as_bytes();
        let len = bytes.len().min(15);
        duck.name[..len].copy_from_slice(&bytes[..len]);
        duck
    }

    /// Get vtable pointer for ISwimmer interface
    pub fn vtable_ptr_i_swimmer() -> *const ISwimmerVTable {
        &__DUCK_ISWIMMER_VTABLE
    }

    /// Get vtable pointer for IFlyer interface
    pub fn vtable_ptr_i_flyer() -> *const IFlyerVTable {
        &__DUCK_IFLYER_VTABLE
    }

    /// Cast to ISwimmer (primary interface at offset 0)
    pub fn as_swimmer(&self) -> &ISwimmer {
        unsafe { &*(self as *const Self as *const ISwimmer) }
    }

    /// Cast to IFlyer (secondary interface - requires this-adjustment)
    pub fn as_flyer(&self) -> &IFlyer {
        unsafe {
            let ptr = (self as *const Self as *const u8)
                .add(std::mem::offset_of!(Self, vtable_i_flyer));
            &*(ptr as *const IFlyer)
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

    // Test #[slot(N)] attribute
    println!("\n--- Slot index test (proc-macro) ---");
    let slot_tester = SlotTester::new();
    // Verify vtable has correct size: 7 slots (0,1,2,3,4,5,6) * 8 bytes = 56 bytes on x64
    let vtable_size = std::mem::size_of::<ISlotTestVTable>();
    let ptr_size = std::mem::size_of::<*const ()>();
    let expected_slots = 7; // slots 0-6
    let expected_size = expected_slots * ptr_size;
    println!(
        "  ISlotTestVTable size: {} bytes ({} slots)",
        vtable_size,
        vtable_size / ptr_size
    );
    assert_eq!(
        vtable_size, expected_size,
        "VTable should have 7 slots (0-6)"
    );

    // Call methods through the interface to verify they work
    unsafe {
        let iface = &*(&slot_tester as *const SlotTester as *const ISlotTest);
        let iface = std::ptr::from_ref(iface).cast_mut().as_mut().unwrap();
        assert_eq!(iface.method_at_0(), 0, "method_at_0 should return 0");
        assert_eq!(iface.method_at_1(), 1, "method_at_1 should return 1");
        assert_eq!(iface.method_at_5(), 5, "method_at_5 should return 5");
        assert_eq!(iface.method_at_6(), 6, "method_at_6 should return 6");
    }
    println!("  All slot methods called correctly!");

    // =========================================================================
    // TEST: Multiple inheritance
    // =========================================================================
    println!("\n--- Multiple inheritance test ---");

    let duck = Duck::new("Donald");
    println!("Duck struct size: {} bytes", std::mem::size_of::<Duck>());
    println!(
        "  vtable_i_swimmer offset: {}",
        std::mem::offset_of!(Duck, vtable_i_swimmer)
    );
    println!(
        "  vtable_i_flyer offset: {}",
        std::mem::offset_of!(Duck, vtable_i_flyer)
    );

    // Direct method calls
    println!("\nDirect calls:");
    duck.swim();
    duck.fly();
    println!("  swim_speed: {}", duck.swim_speed());
    println!("  fly_altitude: {}", duck.fly_altitude());

    // Polymorphic calls through interfaces
    println!("\nPolymorphic calls through ISwimmer:");
    let swimmer = duck.as_swimmer();
    let swimmer = unsafe { std::ptr::from_ref(swimmer).cast_mut().as_mut().unwrap() };
    unsafe {
        swimmer.swim();
        println!("  swim_speed via interface: {}", swimmer.swim_speed());
    }

    println!("\nPolymorphic calls through IFlyer (this-adjusted):");
    let flyer = duck.as_flyer();
    let flyer = unsafe { std::ptr::from_ref(flyer).cast_mut().as_mut().unwrap() };
    unsafe {
        flyer.fly();
        println!("  fly_altitude via interface: {}", flyer.fly_altitude());
    }

    println!("  Multiple inheritance works!");

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
    println!(
        "  ISlotTestVTable: {} bytes ({} slots)",
        vtable_size,
        vtable_size / ptr_size
    );

    println!("\n=== ALL TESTS PASSED - VTABLE LAYOUTS MATCH! ===");
}
