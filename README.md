# cppvtable

Rust library for C++ vtable interop with MSVC ABI compatibility.

Define C++ compatible interfaces and classes in Rust that can:

- Call methods on C++ objects passed to Rust
- Be passed to C++ code which can call methods through the vtable
- Implement COM interfaces with proper IUnknown support

## Features

- **MSVC ABI compatible** - vtable layout matches MSVC C++ compiler
- **Calling conventions** - `thiscall` on x86, `C` on x64
- **Explicit slot indices** - `[N] fn method()` syntax for specific vtable slots
- **Multiple inheritance** - proper this-pointer adjustment
- **Rust-side RTTI** - `TypeInfo` and `cast_to()` for runtime interface casting
- **COM support** - `#[com_interface]` and `#[com_implement]` for COM interfaces with auto-generated IUnknown
- **Two macro approaches** - declarative (`macro_rules!`) and proc-macro

## Limitations

- **No C++ RTTI support** - This crate does not interoperate with C++ native RTTI (`dynamic_cast`, `typeid`). C++ RTTI uses complex ABI-specific structures that vary between MSVC and GCC/Clang. If you need runtime casting of C++ objects, the C++ code should expose its own casting mechanism. The `rtti` module provides Rust-side type info for casting between interfaces on Rust objects only.

## Usage

### COM Interfaces

```rust
use cppvtable::com::{ComRefCount, HRESULT, S_OK};
use cppvtable::proc::{com_interface, com_implement};

// Define a COM interface (automatically extends IUnknown)
#[com_interface("12345678-1234-5678-9abc-def012345678")]
pub trait ICalculator {
    fn add(&self, a: i32, b: i32) -> i32;
    fn multiply(&self, a: i32, b: i32) -> i32;
}

// Implement the interface
#[repr(C)]
pub struct Calculator {
    vtable_i_calculator: *const ICalculatorVTable,
    ref_count: ComRefCount,
    base_value: i32,
}

#[com_implement(ICalculator)]
impl Calculator {
    fn add(&self, a: i32, b: i32) -> i32 {
        self.base_value + a + b
    }
    fn multiply(&self, a: i32, b: i32) -> i32 {
        self.base_value * a * b
    }
    // IUnknown methods (query_interface, add_ref, release) are auto-generated
}
```

### Proc-Macros (Non-COM)

```rust
use cppvtable::proc::{cppvtable, cppvtable_impl};

#[cppvtable]
pub trait IAnimal {
    fn speak(&self);
    fn legs(&self) -> i32;
}

#[repr(C)]
pub struct Dog {
    vtable: *const IAnimalVTable,
    pub name: [u8; 32],
}

#[cppvtable_impl(IAnimal)]
impl Dog {
    fn speak(&self) {
        println!("Woof!");
    }
    fn legs(&self) -> i32 {
        4
    }
}
```

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

| Feature           | Declarative        | Proc-macro      | COM               |
| ----------------- | ------------------ | --------------- | ----------------- |
| Slot indices      | ✅ `[N] fn method` | ✅ `#[slot(N)]` | ✅ (auto)         |
| thiscall (x86)    | ✅                 | ✅              | ✅ (stdcall)      |
| IUnknown support  | ❌                 | ❌              | ✅ (auto)         |
| Interface IID     | ❌                 | ❌              | ✅ (GUID)         |
| Clean Rust syntax | ❌                 | ✅              | ✅                |

## Project Structure

```
cppvtable/
├── Cargo.toml              # Workspace root
└── crates/
    ├── cppvtable/          # Main library (pure Rust)
    │   └── src/
    │       ├── lib.rs      # Re-exports both approaches
    │       ├── decl.rs     # Declarative macros
    │       ├── com.rs      # COM types (GUID, HRESULT, IUnknown)
    │       └── rtti.rs     # Rust-side RTTI for interface casting
    ├── cppvtable-macro/    # Proc-macro crate
    │   └── src/
    │       └── lib.rs      # #[cppvtable], #[cppvtable_impl], #[com_interface], #[com_implement]
    └── cppvtable-cpp-tests/ # C++ interop tests (requires MSVC)
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

**Test coverage (87 tests):**

- Single & multiple inheritance
- This-pointer adjustment for secondary interfaces
- Rust calling C++ objects, C++ calling Rust objects
- TypeInfo/RTTI: `implements()`, `cast_to()`, null for unknown interfaces
- VTable layout verification against MSVC
- COM interfaces: IID generation, QueryInterface, AddRef/Release, interface inheritance

## Requirements

- Rust 2024 edition
- MSVC toolchain (only for `cppvtable-cpp-tests`)

## License

MIT
