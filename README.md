# cppvtable

Call and implement C and C++ virtual-table interfaces in Rust.
COM is a separate library, `cppvtable-com`.

## Crate boundaries

| Crate | Responsibility |
| --- | --- |
| `cppvtable-abi` | Shared pointer/vtable metadata and borrowed C/C++ call wrappers; no object ownership or COM types. |
| `cppvtable` | C/C++ interface declarations, Rust implementations, and owned objects. |
| `cppvtable-com` | COM declarations, GUID/HRESULT, `IUnknown`, `QueryInterface`, owning COM pointers, and reference-count policies. |
| `cppvtable-macro` | Procedural macros reexported by the public libraries. |

Both object libraries depend on `cppvtable-abi`. Neither depends on the other.
`cppvtable` has no COM feature, COM namespace, GUID, or reference-count requirement.

## Implement and call a C++ interface

```rust
use cppvtable::{OwnedObject, implement, interface};

#[interface(abi = cpp)]
pub unsafe trait IAnimal {
    /// Return the number of legs.
    fn legs(&self) -> u32;
}

#[implement(IAnimal)]
struct Dog;

impl IAnimalImpl for Dog {
    fn legs(&self) -> u32 {
        4
    }
}

fn main() {
    let dog = OwnedObject::new(Dog);
    let animal = dog.interface::<IAnimal>();
    // SAFETY: The owner keeps the object alive and the method has no preconditions.
    assert_eq!(unsafe { animal.legs() }, 4);

    // Pass this borrowed pointer to C++ code expecting IAnimal*.
    let _raw = dog.as_raw::<IAnimal>();
}
```

The matching C++ interface is:

```cpp
struct IAnimal {
    virtual unsigned int legs() = 0;
};
```

`OwnedObject<T>` allocates stable storage containing vtable pointers followed by the
Rust value. Dropping the owner destroys the Rust value exactly once. Interface
borrows cannot outlive the owner; foreign code using a raw pointer must obey the
same lifetime. C++ must not `delete` the Rust allocation.

Use `#[implement(IFirst, ISecond)]` for multiple interface chains. Each chain has
its own vtable pointer, and generated shims adjust `this` back to the Rust object.
`extends(IBase)` embeds the base vtable as a prefix and exposes inherited methods.
`query_interface::<IBase>()` on the owner looks up a declared interface or ancestor
using Rust type identity; it is not COM `QueryInterface` and changes no counts.

For a foreign-owned object, use `IAnimal::from_raw_ref(&raw)` in an unsafe block.
The pointer must be valid and the foreign owner must outlive the borrow. A project
that only calls foreign objects can use `cppvtable-abi::interface` directly.

## C tables and return values

Select the binary contract explicitly, or use the target defaults:

| `abi` | Binary contract |
| --- | --- |
| `msvc` | Microsoft C++ ABI, including Clang targeting that ABI. |
| `itanium` | Itanium C++ ABI, including Clang targeting that ABI. |
| `cpp` | Default C++ ABI for the Rust compilation target. |
| `c` | Default C ABI for the Rust compilation target. |
| `com` | COM ABI, available through `cppvtable-com`. |

For example, use `#[interface(abi = msvc)]` when binding a specifically Microsoft
C++ interface. `cpp` chooses from the Rust target configuration; macros cannot
detect an independently configured foreign compiler or its flags. Compile the
foreign code for a matching target and ABI.
Explicit `msvc` requires an MSVC Rust target; explicit `itanium` rejects MSVC
targets. These options do not provide cross-ABI calls within one process.

`#[interface(abi = c)]` describes a C-compatible function table. The object begins
with a pointer to that table; each function receives the object pointer first.

For a C header that stores its function pointers directly inside the object, select
`layout = inline`:

```rust
use cppvtable::{OwnedObject, implement, interface};

#[interface(abi = c, layout = inline)]
pub unsafe trait IInlineCounter {
    /// Read the current value.
    fn value(&self) -> u32;
}

#[implement(IInlineCounter)]
struct Counter { value: u32 }

impl IInlineCounterImpl for Counter {
    fn value(&self) -> u32 { self.value }
}

fn main() {
    let owner = OwnedObject::new(Counter { value: 42 });
    let counter = owner.interface::<IInlineCounter>();
    // SAFETY: The owner keeps the inline header and implementation alive.
    assert_eq!(unsafe { counter.value() }, 42);
    assert_eq!(counter.vtable().cast_mut().cast(), counter.as_raw());
}
```

The matching C interface header contains the callback itself:

```c
#include <stdint.h>
struct IInlineCounter {
    uint32_t (*value)(struct IInlineCounter *self);
};
```

The interface pointer addresses that header, so `vtable()` equals the interface
address. `layout = pointer` is the default and addresses an object field containing
a separate table pointer. Inline layout is available for `abi = c`; it supports
`slots`, reserved entries, and `extends` with a base using the same layout.

An implementation can combine independent pointer and inline interface chains.
Each chain occupies its own header, and generated shims adjust their pointers back
to the same Rust value. `OwnedObject` places those headers before its Rust value
and preserves its alignment and lifetime. This does not generate arbitrary C data
field layouts; foreign code accesses the declared interface header. Callback
entries must stay immutable while Rust borrows the interface, including headers
owned by foreign code.

Override individual methods with `#[abi(convention = "system")]`, for example
when a C table combines C and Windows system calls. The override applies to both
the function-pointer field and the Rust implementation shim. On Windows x86,
`"C"`, `"stdcall"`, `"fastcall"`, and `"thiscall"` select distinct conventions.
Use only conventions supported by the Rust target and matching the foreign header.
The override changes the calling convention, while return lowering still follows
the interface ABI and the method's `scalar`, `aggregate`, or `hidden_return` option.
Options can be combined, such as `#[abi(convention = "C", aggregate)]`.

Partial tables can name only the known functions:

```rust
use cppvtable::interface;

#[interface(abi = c, slots = 50)]
pub unsafe trait IPartial {
    /// Read the known value from the 33rd entry.
    #[slot(32)]
    #[abi(convention = "system")]
    fn known(&self) -> u32;
}
```

`#[slot(N)]` uses zero-based function-pointer entries, never byte offsets. It
reserves unknown entries before the named method. Without `slots`, the generated
table ends immediately after the last declared entry; `slots = 50` also reserves
unknown trailing entries so the entire table contains 50 entries. Unknown entries
need no invented signatures or method names.

For `extends(IBase)`, method slot indices start after the complete base vtable,
while `slots` counts the complete table including that base. The compiler rejects
an extent smaller than the inherited and declared entries. Reserved entries in a
generated Rust implementation are null and must not be called. A partial declaration
can describe a foreign object with additional working methods; the Rust implementation
only provides its declared methods.

Use fixed-layout FFI types in method signatures, raw pointers for borrowed data,
and interior mutability when implementations change state. Interface methods use
`&self` because callbacks can reenter the object.

The declaration's `fn` or `unsafe fn` is preserved in the generated implementation
trait. Declare a method `unsafe fn` and document its `# Safety` requirements when
it requires valid pointer arguments or assumes that `self` is embedded in an
allocated object. A safe implementation method must also be safe when called
directly from Rust on a standalone implementation value. Merely declaring an
`unsafe trait` does not make its methods unsafe.

Calls through the generated foreign-interface wrapper always require `unsafe`,
even when the implementation method is safe. The generated vtable shim recovers
the embedded implementation value; a direct Rust call to an implementation method
does not establish that allocation invariant. Helpers such as `interface_of` and
COM `from_impl` require the caller to establish it explicitly.

- Scalars and pointers use the selected calling convention directly.
- `#[abi(scalar)]` permits a transparent scalar wrapper.
- `#[abi(aggregate)]` returns a trivially copyable `#[repr(C)]` aggregate using the
  target's C/C++ return convention, including small values returned in registers.
- `#[abi(hidden_return)]` explicitly describes an indirect return. Use it only
  when the foreign method actually uses that convention; it is not a general
  substitute for `aggregate`.

The declaration must match the foreign header, including slot order, argument
layout, and return convention. Rust panics and C++ exceptions must not cross the
FFI boundary.

## COM

Depend on `cppvtable-com` directly:

```rust
use cppvtable_com::{
    ComObject, HRESULT, RefCounted, S_OK, SingleRefCount, implement, interface,
};

#[interface(abi = com, iid = "1c1a0b4f-2a4a-4a1b-9a4a-0f0a0b0c0d01")]
pub unsafe trait IThing {
    /// Write the value to the caller's storage.
    ///
    /// # Safety
    ///
    /// `value` must point to aligned, writable storage for a `u32`.
    unsafe fn GetValue(&self, value: *mut u32) -> HRESULT;
}

#[implement(IThing)]
struct Thing {
    value: u32,
}

// SAFETY: Uses the standalone policy and no custom pointer-returning hooks.
unsafe impl RefCounted for Thing {
    type Policy = SingleRefCount;
}

impl IThingImpl for Thing {
    unsafe fn GetValue(&self, value: *mut u32) -> HRESULT {
        // SAFETY: The interface contract requires writable output storage.
        unsafe { *value = self.value };
        S_OK
    }
}

fn main() {
    let thing = ComObject::new(Thing { value: 7 });
    let mut value = 0;
    // SAFETY: `value` is writable storage.
    assert!(unsafe { thing.GetValue(&raw mut value) }.is_ok());
    assert_eq!(value, 7);
}
```

`ComPtr<I>` owns one public reference: cloning calls `AddRef`, and dropping calls
`Release`. `SingleRefCount`, `DualRefCount`, and `ForwardRefCount` provide standalone,
public/private, and container-forwarded lifetimes. `cppvtable_com::OwnedObject`
is specifically for container-owned COM children; ordinary objects use
`cppvtable::OwnedObject`.

COM pointers are not automatically thread-safe. Implementing the unsafe
`AgileInterface` marker opts an interface into cross-thread use and requires all
implementations of that interface to satisfy its threading contract. Custom
`RefCounted` hooks and container-owned children must satisfy their documented
ownership contracts.

Enable `cppvtable-com/windows-compat` to use `windows-core`'s `GUID` and `HRESULT`.
This feature does not change either ordinary C/C++ crate.

## Compiler support and scope

The native compiler test matrix is:

| Platform | Compiler | ABI |
| --- | --- | --- |
| Windows x86 and x86-64 | MSVC | Microsoft C++ |
| Windows x86 and x86-64 | clang-cl | Microsoft C++ |
| Linux x86-64 | clang / clang++ | C / Itanium C++ |

Other architectures are not covered by this matrix. The target determines calling conventions:
Microsoft x86 C++ uses `thiscall`; other supported C++ targets use the platform C
calling convention. COM uses `system`, and C tables use `C`.

This library implements declared virtual interface contracts. It does not generate
arbitrary C++ class layouts, RTTI, virtual inheritance, constructor/destructor
protocols, covariant-return thunks, or C++ exception interoperability. Use explicit
base interface pointers for foreign multiple inheritance. Rust-created interfaces
must not be used with C++ `dynamic_cast`, `typeid`, or `delete`.

ABI references: [Clang's Microsoft ABI compatibility](https://clang.llvm.org/docs/MSVCCompatibility.html)
and the [Itanium C++ ABI](https://itanium-cxx-abi.github.io/cxx-abi/abi.html).

## Build and test

Rust 1.85 or later is required. Native integration tests and the example also need
a C/C++ compiler and its platform SDK. Set `CXX` to select a compiler, for example
`clang-cl` on Windows or `clang++` on Linux.

```sh
cargo test --workspace
cargo test --workspace --all-features
cargo test --workspace --release
cargo run -p cppvtable-example
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo deny check
```

To test without a native compiler, select `cppvtable`, `cppvtable-abi`,
`cppvtable-com`, and `cppvtable-macro` with Cargo's `-p` options.

The tests cover borrowed foreign objects, Rust implementations, C tables, inherited
and secondary interfaces, aggregate returns, COM identity, and reference-count
lifetime transitions. Compiler jobs are defined in `.github/workflows`.

## Migration from the partial refactor

- Replace `cppvtable::com::*` with `cppvtable_com::*` and add that direct dependency.
- Move COM identifiers and `windows-compat` usage to `cppvtable-com`.
- Use root `cppvtable::{interface, implement, OwnedObject}` for regular objects;
  remove their COM `RefCounted` implementations.
- COM `RefCounted` implementations now require `unsafe impl` and adherence to the
  documented hook contracts. Lifecycle, container, and query hooks use `unsafe fn`
  because the runtime supplies a live embedded object to them.
- Interface methods with pointer-validity or object-allocation preconditions must
  be declared and implemented as `unsafe fn`. The macro preserves that qualifier;
  direct Rust calls must uphold the documented preconditions.

## License

MIT.
