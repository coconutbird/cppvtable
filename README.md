# cppvtable

Rust library for C++ vtable interop with MSVC ABI compatibility.

Define C++ compatible interfaces and classes in Rust that can:
- Call methods on C++ objects passed to Rust
- Be passed to C++ code which can call methods through the vtable

## Features

- **MSVC ABI compatible** - vtable layout matches MSVC C++ compiler
- **Calling conventions** - `thiscall` on x86, `C` on x64
- **Explicit slot indices** - `[N] fn method()` syntax for specific vtable slots
- **Multiple inheritance** - proper this-pointer adjustment
- **Two macro approaches** - declarative (`macro_rules!`) and proc-macro

## Usage

### Declarative Macros

```rust
use cppvtable::{define_interface, define_class};

define_interface! {
    interface IAnimal {
        fn speak(&self);
        fn legs(&self) -> i32;
    }
}

define_interface! {
    interface IAdvancedAnimal : IAnimal {
        fn run(&mut self);
        [5] fn special_method(&self);  // explicit slot index
    }
}

define_class! {
    pub class Dog : IAnimal {
        pub name: [u8; 32],
    }
}
```

### Proc-Macros

```rust
use cppvtable::proc::{cpp_interface, implement};

#[cpp_interface]
pub trait IAnimal {
    fn speak(&self);
    fn legs(&self) -> i32;
}

#[repr(C)]
pub struct Dog {
    vtable: *const IAnimalVTable,
    pub name: [u8; 32],
}

#[implement(IAnimal)]
impl Dog {
    fn speak(&self) {
        println!("Woof!");
    }
    fn legs(&self) -> i32 {
        4
    }
}
```

### Consuming C++ Objects

```rust
// Pointer from C++ code
let cpp_animal: *mut c_void = get_cpp_animal();

// Wrap and call methods
unsafe {
    let animal = IAnimal::from_ptr_mut(cpp_animal);
    animal.speak();
    println!("Legs: {}", animal.legs());
}
```

## Feature Comparison

| Feature | Declarative | Proc-macro |
|---------|-------------|------------|
| Slot indices `[N]` | ✅ | ❌ (planned) |
| thiscall (x86) | ✅ | ❌ (planned) |
| Clean Rust syntax | ❌ | ✅ |
| No separate crate | ✅ | N/A |

## Project Structure

```
cppvtable/
├── Cargo.toml           # Workspace + examples binary
├── src/main.rs          # Examples and C++ interop tests
└── crates/
    ├── cppvtable/       # Library crate
    │   └── src/
    │       ├── lib.rs   # Re-exports both approaches
    │       └── decl.rs  # Declarative macros
    └── cppvtable-macro/ # Proc-macro crate
        └── src/
            └── lib.rs   # #[cpp_interface], #[implement]
```

## Testing

The project includes C++ interop tests using the `cpp` crate to verify vtable layout compatibility with actual MSVC-compiled C++ code:

```bash
cargo run
```

This compiles inline C++ classes and tests bidirectional vtable calls:
- Rust calling C++ objects via `from_ptr()`
- C++ calling Rust objects through generated vtables

## Requirements

- Rust 2024 edition
- MSVC toolchain (for C++ interop tests)

## License

MIT

