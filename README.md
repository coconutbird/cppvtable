# cppvtable

Rust ABI support for COM interfaces, C++ vtables, and C tables of function pointers.

`cppvtable` generates interface types, vtables, and object shims from Rust traits. It handles calling conventions, interface inheritance, `this`-pointer adjustment for multiple interfaces, COM identity, and reference-count policies.

## Declare an interface

Use `#[interface]` with `abi = com`, `cpp`, or `c`:

```rust
use cppvtable::{HRESULT, interface};

#[interface(abi = com, iid = "1c1a0b4f-2a4a-4a1b-9a4a-0f0a0b0c0d01")]
pub unsafe trait IThing {
    /// Write the value of the thing.
    fn GetValue(&self, value: *mut u32) -> HRESULT;
}
```

COM interfaces require an IID and default to `IUnknown` as their base. C++ interfaces use the C++ method calling convention (`thiscall` on x86, C elsewhere); C interfaces use `extern "C"`. Use `extends(IBase)` to declare an interface base and `#[slot(N)]` for an explicit method slot.

## Implement an interface

Continuing the interface declaration above, mark the object with `#[implement]`, implement the generated `IThingImpl` trait, and choose a reference-count policy:

```rust
use cppvtable::{ComObject, RefCounted, SingleRefCount, S_OK, implement};

#[implement(IThing)]
pub struct Thing {
    value: u32,
}

impl RefCounted for Thing {
    type Policy = SingleRefCount;
}

impl IThingImpl for Thing {
    fn GetValue(&self, value: *mut u32) -> HRESULT {
        // SAFETY: The interface caller provides writable storage.
        unsafe { *value = self.value };
        S_OK
    }
}

fn main() {
    let thing = ComObject::new(Thing { value: 7 });
    let mut value = 0;
    // SAFETY: `value` is writable storage.
    let result = unsafe { thing.GetValue(&raw mut value) };
    assert!(result.is_ok());
    assert_eq!(value, 7);
}
```

`ComPtr<I>` owns a public reference when `I` is a COM interface. For `cpp` and `c` interfaces it is only a pointer wrapper: it does not keep a foreign object alive or change a reference count. Borrow a foreign interface pointer with `I::from_raw_ref` while its owner keeps the object alive. `OwnedObject<T>` owns a Rust-allocated object using the forwarding reference-count policy, typically as a child of a container. The `refcount` module provides single, dual, and forwarding policies.

## Build and test

```sh
# Pure Rust library tests
cargo test -p cppvtable

# C++ interoperability tests and executable example (requires MSVC)
cargo test -p cppvtable-cpp-tests
cargo run -p cppvtable-example

# Workspace checks
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo deny check
```

The C++ interoperability tests cover Rust-to-C++ and C++-to-Rust calls, single and multiple inheritance, and secondary-interface pointer adjustment against MSVC's ABI.

## Workspace layout

- `crates/cppvtable`: public ABI, object, pointer, and reference-count APIs, with Rust integration tests.
- `crates/cppvtable-macro`: the `#[interface]` and `#[implement]` procedural macros.
- `crates/cppvtable-cpp-tests`: MSVC C++ interoperability tests.
- `examples/cppvtable`: a bidirectional C++/Rust interface example.

## Requirements

- Rust 1.85 or later (edition 2024).
- MSVC for the C++ test package and example.

## License

MIT.

