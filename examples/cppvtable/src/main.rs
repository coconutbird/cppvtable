//! A bidirectional C++ ABI example for `cppvtable`.
//!
//! Rust declares the C++ interface with `#[interface(abi = cpp)]`. The inline C++ code
//! implements it once in C++ and also calls a Rust object that implements the same
//! interface. `cpp_build` compiles the inline C++ with the example.

use cpp::cpp;
use cppvtable::{ForwardRefCount, OwnedObject, RefCounted, implement, interface};
use std::ffi::c_void;
use std::io::{self, Write};

/// The C++ interface shared by the C++ and Rust objects below.
#[interface(abi = cpp)]
unsafe trait IAnimal {
    fn speak(&self);
    fn legs(&self) -> i32;
}

#[implement(IAnimal)]
struct RustDog {
    name: String,
}

impl RefCounted for RustDog {
    type Policy = ForwardRefCount;
}

impl IAnimalImpl for RustDog {
    fn speak(&self) {
        println!("RustDog '{}' says: Woof from Rust!", self.name);
    }

    fn legs(&self) -> i32 {
        4
    }
}

cpp! {{
    #include <cstddef>
    #include <cstdio>
    #include <string>

    // Keep this method order and signature in sync with the Rust interface above.
    // There is intentionally no virtual destructor: it would add another vtable slot.
    class IAnimal {
    public:
        virtual void speak() = 0;
        virtual int legs() = 0;
    };

    class CppDog final : public IAnimal {
        std::string name;

    public:
        CppDog(const char* name, std::size_t length) : name(name, length) {}

        void speak() override {
            std::printf("CppDog '%s' says: Woof from C++!\n", name.c_str());
        }

        int legs() override {
            return 4;
        }
    };
}}

fn create_cpp_dog(name: &str) -> *mut c_void {
    let name_ptr = name.as_ptr();
    let name_len = name.len();
    cpp!(unsafe [name_ptr as "const char*", name_len as "size_t"] -> *mut c_void as "void*" {
        return new CppDog(name_ptr, name_len);
    })
}

unsafe fn delete_cpp_dog(animal: *mut c_void) {
    cpp!(unsafe [animal as "IAnimal*"] {
        // This helper only receives the CppDog allocated by `create_cpp_dog`.
        delete static_cast<CppDog*>(animal);
    });
}

unsafe fn call_cpp_animal(animal: *mut c_void) -> i32 {
    cpp!(unsafe [animal as "IAnimal*"] -> i32 as "int" {
        animal->speak();
        std::fflush(stdout);
        return animal->legs();
    })
}

fn main() {
    println!("--- Rust calling a C++ implementation ---");
    let cpp_dog = create_cpp_dog("Max");
    let _ = io::stdout().flush();
    // SAFETY: `cpp_dog` is a live pointer to the matching C++ `IAnimal` vtable.
    let cpp_dog_ref = unsafe { IAnimal::from_raw_ref(&cpp_dog) };
    // SAFETY: The C++ object is alive and its virtual methods obey this interface.
    let cpp_legs = unsafe {
        cpp_dog_ref.speak();
        cpp_dog_ref.legs()
    };
    println!("Rust sees the C++ dog's legs: {cpp_legs}");
    // SAFETY: This pointer came from `create_cpp_dog` and has not been deleted yet.
    unsafe { delete_cpp_dog(cpp_dog) };

    println!("\n--- C++ calling a Rust implementation ---");
    let rust_dog = OwnedObject::new(RustDog {
        name: "Buddy".to_owned(),
    });
    let rust_dog_ptr = rust_dog.as_raw::<IAnimal>();
    let _ = io::stdout().flush();
    // SAFETY: The pointer is a live Rust implementation of the shared interface.
    let rust_legs = unsafe { call_cpp_animal(rust_dog_ptr) };
    println!("C++ sees the Rust dog's legs: {rust_legs}");
}
