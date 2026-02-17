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
- **Rust-side RTTI** - `TypeInfo` and `cast_to()` for runtime interface casting
- **Two macro approaches** - declarative (`macro_rules!`) and proc-macro

## Limitations

- **No C++ RTTI support** - This crate does not interoperate with C++ native RTTI (`dynamic_cast`, `typeid`). C++ RTTI uses complex ABI-specific structures that vary between MSVC and GCC/Clang. If you need runtime casting of C++ objects, the C++ code should expose its own casting mechanism. The `rtti` module provides Rust-side type info for casting between interfaces on Rust objects only.

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

| Feature            | Declarative | Proc-macro   |
| ------------------ | ----------- | ------------ |
| Slot indices `[N]` | ✅          | ❌ (planned) |
| thiscall (x86)     | ✅          | ❌ (planned) |
| Clean Rust syntax  | ❌          | ✅           |
| No separate crate  | ✅          | N/A          |

## Project Structure

```
cppvtable/
├── Cargo.toml              # Workspace root
└── crates/
    ├── cppvtable/          # Main library (pure Rust, 63 tests)
    │   └── src/
    │       ├── lib.rs      # Re-exports both approaches
    │       ├── decl.rs     # Declarative macros
    │       └── rtti.rs     # Rust-side RTTI for interface casting
    ├── cppvtable-macro/    # Proc-macro crate
    │   └── src/
    │       └── lib.rs      # #[cpp_interface], #[implement]
    └── cppvtable-cpp-tests/ # C++ interop tests (requires MSVC, 12 tests)
        └── src/
            ├── lib.rs      # C++ classes, helpers, Rust interfaces
            ├── single.rs   # Single inheritance tests
            └── multi.rs    # Multiple inheritance tests
```

## Testing

```bash
# Run all Rust tests (no C++ compiler needed)
cargo test -p cppvtable

# Run C++ interop tests (requires MSVC)
cargo test -p cppvtable-cpp-tests

# Run all tests
cargo test --workspace
```

**Test coverage (75 total):**
- Single & multiple inheritance
- This-pointer adjustment for secondary interfaces
- Rust calling C++ objects, C++ calling Rust objects
- TypeInfo/RTTI: `implements()`, `cast_to()`, null for unknown interfaces
- VTable layout verification against MSVC

## Requirements

- Rust 2024 edition
- MSVC toolchain (only for `cppvtable-cpp-tests`)

## License

MIT
