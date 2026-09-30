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
    assert_eq!(animal.legs(), 4);

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

`#[interface]` requires an `unsafe trait`. The `unsafe` is the declarer's proof
obligation: slot order, signatures, calling conventions, and return lowering match
the foreign header; every method declared as a safe `fn` has no precondition beyond
a live object; and no method unwinds. In return, a declared `fn` becomes a safe
caller, and a declared `unsafe fn` becomes an `unsafe` caller.

`OwnedObject<T>` allocates stable storage containing vtable pointers followed by the
Rust value and dereferences to `T`. Dropping the owner destroys the Rust value exactly
once. `interface::<I>()` returns an `InterfaceRef<'_, I>`, a pointer-sized borrow that
dereferences to `I` and cannot outlive the owner; foreign code using a raw pointer
must obey the same lifetime. C++ must not `delete` the Rust allocation.

Use `#[implement(IFirst, ISecond)]` for multiple interface chains. Each chain has
its own vtable pointer, and generated shims adjust `this` back to the Rust object.
`extends(IBase)` embeds the base vtable as a prefix and exposes inherited methods.
`try_interface::<IBase>()` on the owner looks up a declared interface or ancestor
using Rust type identity and returns `Option<InterfaceRef<'_, IBase>>`; it is not COM
`QueryInterface` and changes no counts. `#[implement]` also accepts tuple structs.

For a foreign-owned object, `unsafe { IAnimal::from_raw(raw) }` returns
`Option<InterfaceRef<'_, IAnimal>>` (`None` for null), and `from_non_null` takes a
`NonNull<c_void>`. The pointer must be valid and the foreign owner must outlive the
borrow. `Option<InterfaceRef<'_, I>>` is pointer-sized, so it can also be an FFI
parameter type for a nullable borrowed interface pointer. Interface newtypes are
neither `Clone` nor `Copy`; they implement `Debug` and compare by pointer identity.
`vtable()` returns `&IAnimalVtbl`, borrowed from the interface. A project that only
calls foreign objects can use `cppvtable-abi::interface` directly.

Non-doc outer attributes on the trait are forwarded to the generated newtype;
`deprecated`, `must_use`, `allow`, and `expect` on a method are forwarded to both the
caller and the `Impl` method. `cfg` on methods is rejected.

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
    assert_eq!(counter.value(), 42);
    assert_eq!(core::ptr::from_ref(counter.vtable()).cast(), counter.as_raw().cast_const());
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
`"C"`, `"stdcall"`, `"fastcall"`, and `"thiscall"` select distinct conventions; on
other architectures these four lower to `"C"`, so one declaration serves every
target. Use only conventions matching the foreign header.
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
trait and in the caller. Declare a method `unsafe fn` and document its `# Safety`
requirements when it requires valid pointer arguments or assumes that `self` is
embedded in an allocated object. A safe implementation method must also be safe when
called directly from Rust on a standalone implementation value.

Calling a declared `fn` through an interface value is safe: the value exists only
for a live object, and the `unsafe trait` proves the rest. The generated vtable shim
recovers the embedded implementation value; a direct Rust call to an implementation
method does not establish that allocation invariant. Helpers such as `interface_of`
and COM `from_impl` require the caller to establish it explicitly.

To build a table or hook entry by hand, `#[vtable_fn(abi = cpp|c|msvc|itanium|com)]`
on a free `unsafe fn` emits it with exactly the convention the vtable fields of that
ABI use on each target; `convention = "..."` matches a method override. The signature
is the lowered one, with `this` first:

```rust
use core::ffi::c_void;
use cppvtable::{interface, vtable_fn};

#[interface(abi = cpp)]
pub unsafe trait ICounter {
    fn value(&self) -> u32;
}

#[vtable_fn(abi = cpp)]
unsafe fn value(_this: *mut c_void) -> u32 {
    7
}

fn main() {
    let _table = ICounterVtbl { value };
}
```

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
use cppvtable_com::{ComObject, ComPtr, HRESULT, IUnknown, implement, interface, write_out};

#[interface(abi = com, iid = "1c1a0b4f-2a4a-4a1b-9a4a-0f0a0b0c0d01")]
pub unsafe trait IThing {
    /// Return the stored value.
    fn Value(&self) -> u32;

    /// Write a new `IUnknown` reference to this object into the caller's slot.
    ///
    /// # Safety
    ///
    /// `out` must be null or aligned and writable for one `Option<ComPtr<IUnknown>>`,
    /// and `self` must be embedded in a live `ComObject`.
    unsafe fn GetUnknown(&self, out: *mut Option<ComPtr<IUnknown>>) -> HRESULT;
}

#[implement(IThing, refcount = single)]
struct Thing {
    value: u32,
}

impl IThingImpl for Thing {
    fn Value(&self) -> u32 {
        self.value
    }

    unsafe fn GetUnknown(&self, out: *mut Option<ComPtr<IUnknown>>) -> HRESULT {
        // SAFETY: The method contract embeds `self` in a live `ComObject`.
        let this = unsafe { ComPtr::<IThing>::from_impl(self) };
        // SAFETY: The method contract gives a null or writable output slot.
        unsafe { write_out(out, this.cast::<IUnknown>()) }
    }
}

fn main() {
    let thing = ComObject::new(Thing { value: 7 });
    assert_eq!(thing.Value(), 7);

    let mut unknown = None;
    // SAFETY: `unknown` is writable storage for the out-parameter.
    assert!(unsafe { thing.GetUnknown(&raw mut unknown) }.is_ok());
    assert!(unknown.is_some());

    let again = ComPtr::from_ref(&*thing);
    assert_eq!(again.cast::<IThing>().map(|p| p.Value()), Some(7));
}
```

`ComPtr<I>` owns one public reference: cloning calls `AddRef`, and dropping calls
`Release`. `ComPtr::from_ref(&iface)` adds a reference to a borrowed interface.
`.cast::<J>()` is an inherent `IUnknown` method reached through `Deref` on any
interface or `ComPtr`; it calls `QueryInterface` and returns an owned
`Option<ComPtr<J>>`. Declare interface out-parameters as `*mut Option<ComPtr<I>>`,
which has the ABI of `I**`, and fill them with `write_out`, which returns `E_POINTER`
for a null slot and `S_OK` otherwise.

`#[implement(..., refcount = single | dual)]` selects `SingleRefCount` (standalone)
or `DualRefCount` (public/private) and implements `RefCounted`. If an implemented
interface is an `AgileInterface`, the type must be `Send + Sync`. `ForwardRefCount`
(container-forwarded lifetime) and custom hooks still use a manual
`unsafe impl RefCounted`. `cppvtable_com::ChildObject` is specifically for
container-owned COM children; ordinary objects use `ComObject`, and non-COM objects
use `cppvtable::OwnedObject`. `PrivateRef::from_raw_add_ref` adds a private reference
from a raw interface pointer.

COM pointers are not automatically thread-safe. Implementing the unsafe
`AgileInterface` marker opts an interface into cross-thread use and requires all
implementations of that interface to satisfy its threading contract. Custom
`RefCounted` hooks and container-owned children must satisfy their documented
ownership contracts.

Enable `cppvtable-com/windows-compat` to use `windows-core`'s `GUID` and `HRESULT`.
This substitution applies on Windows. Other targets retain the crate's local
representations, including when all features are enabled. This feature does not
change either ordinary C/C++ crate.

`windows-compat` accepts every windows-core release from 0.50 through 0.100, so
Cargo unifies it with the version your `windows` or `windows-core` dependency
selects and the types are interchangeable with that crate's. The newest releases
need a newer Rust than this crate's MSRV; the MSRV-aware resolver picks an older
one on older toolchains. CI tests 0.50 on the MSRV and the newest release on stable.

## Using the libraries without std

`cppvtable-abi`, `cppvtable`, and `cppvtable-com` are `no_std` libraries. The ABI
crate needs no allocator; the two object libraries use `alloc::boxed::Box`, so
applications allocating Rust implementations must supply a global allocator.
COM reference-count policies also require target support for 32-bit atomics.
Procedural macros execute on the build host and do not add a target `std` dependency.

The maintained consumer in `tests/no-std` compiles and exercises generated pointer,
inline, and COM interfaces, and type-checks the RTTI classes and `VtableHook`. Its
CI checks all features on `thumbv7em-none-eabi`:

```sh
cargo check -p cppvtable-abi -p cppvtable -p cppvtable-com --all-features --target thumbv7em-none-eabi
cargo build --manifest-path tests/no-std/Cargo.toml --all-features --target thumbv7em-none-eabi
```

## C++ RTTI

RTTI is opt-in for Rust-created objects. Capture MSVC or Itanium metadata from a
matching native C++ class with `unsafe { RttiMetadata::of(&*iface) }`, which uses the
interface's declared C++ ABI; `RttiMetadata::from_interface(CppAbi::TARGET,
native_pointer)` is the raw form, and `CppAbi::TARGET` is the Rust target's default
C++ ABI. Then build an `RttiClass<T>` once, supplying metadata for each C++ interface
with `with::<I>`; C interfaces take none. `RttiObject::new(value, &class)` allocates
objects whose headers point at the class's shared, prefixed callback tables, so each
object costs the same as `OwnedObject::new`:

```rust,ignore
// SAFETY: `native` is a live object of a native class matching `Widget`'s interfaces.
let metadata = unsafe { RttiMetadata::of(&*native) };
// SAFETY: `metadata` describes a native class matching `Widget`'s interfaces.
let class = unsafe { RttiClass::<Widget>::builder().with::<IWidget>(metadata).build() }?;
let first = RttiObject::new(Widget::default(), &class);
let second = RttiObject::new(Widget::default(), &class);
```

Each `RttiObject` borrows its class, so the class cannot be dropped while objects
exist. It dereferences to `OwnedObject<T>`; `into_raw` and `RttiObject::from_raw`
transfer ownership, and the caller keeps the class alive meanwhile.

ABI families and RTTI representations are separate. Use
`RttiMetadata::from_interface_variant` for an explicit `RttiVariant`:

| Representation | Support |
| --- | --- |
| `MsvcAbsolute` | Absolute-pointer locator revision 0; native x86 tests. |
| `MsvcImageRelative` | Image-relative locator revision 1; native x64 tests. |
| `ItaniumPointer` | Pointer-sized entries and ordinary type-info names; native Linux Clang tests. |
| `ItaniumAppleArm64` | Unsigned pointer tables and tagged type-info name pointers; wire-format tests, no native Apple validation. |
| `ItaniumRelative32` | Clang signed 32-bit table components and RTTI proxies with ordinary untagged type names; native inspection and explicit function-resolution tests. |

The MSVC convenience constructor reads the locator revision. The Itanium convenience
constructor selects the ordinary target-default pointer representation; it cannot
discover a foreign compiler's relative-vtable flags. Generated interface tables and
RTTI-enabled Rust objects currently use pointer entries. Relative32 objects require
the explicit `relative_function` resolver; relative metadata is rejected by
`build` instead of being attached to an incompatible pointer table.

Apple arm64e pointer authentication requires compiler-specific signing and
authentication adapters and is not covered by the unsigned Apple representation.
Actual IA-64 function-descriptor tables are also outside the supported table model.
ARM32's exception-table RTTI relocations do not change ordinary object vtables into
Clang's relative32 representation. These are separate contracts, not aliases for
`itanium`.

The native class supplies the real type identity and inheritance graph. C++ can
then use `typeid`, downcasts, cross-casts, and `dynamic_cast<void*>` on those Rust
objects. `build` checks the ABI, complete-type identity, subobject offsets, and
Microsoft construction-displacement and virtual-inheritance flags, and reports a C++
interface without metadata as `RttiError::InterfaceKind`. `with::<I>` rejects C
interfaces at compile time. `build` is unsafe because the caller must also match the
complete nonvirtual inheritance graph and every declared method contract; Itanium
virtual inheritance cannot be detected without the C++ runtime and remains the
caller's responsibility.

RTTI-enabled Rust objects and shadow-hooked objects point at tables that are not the
compiler's own. Clang 17 and later compile `dynamic_cast` to a `final` class as a
comparison against the compiler's table address, which fails for these objects, so
build C++ that casts them to `final` classes with `-fno-assume-unique-vtables`.
MSVC and clang-cl always call the runtime.

Native descriptors are treated as C++ treats `type_info`: static storage whose
module stays loaded while it is used. Module unloading is not modeled; the source
table and relative proxy only need to remain readable during extraction. Relative32
tables combined with Apple arm64 tagged-name encoding are not currently supported.

The allocation-free `cppvtable_abi::rtti` APIs also inspect native type identity,
encoded names, and complete-object addresses. Explicit runtime function-pointer
adapters enable Rust-side dynamic casts without forcing a C++ runtime dependency
on every `no_std` consumer. The opt-in `native-dynamic-cast` feature of
`cppvtable-abi`, forwarded by `cppvtable`, adds `DynamicCastRuntime::TARGET`, which
links the target runtime's own cast (`__dynamic_cast` or `__RTDynamicCast`); the final
link must include the C++ runtime, and an exception escaping it aborts the process.
Native foreign objects retain their compiler's RTTI behavior, including virtual
inheritance; creating virtual base layouts in Rust is outside this library's object
model.

`OwnedObject::new` retains ordinary dispatch-only tables and must not be passed to
C++ RTTI operations. RTTI does not authorize C++ access to undeclared data members,
construction/destruction, or `delete` of a Rust allocation. Callback calls must stay
virtual through the declared interfaces; native final or devirtualized method bodies
cannot replace Rust callbacks. A native bridge whose bodies forward into Rust is
required when callers rely on those direct implementation calls.

The [native RTTI fixtures](crates/cppvtable-cpp-tests/src/rtti.rs) show the matching
C++ classes, metadata extraction, Rust implementations, and runtime cast adapters.
The [relative32 fixture](crates/cppvtable-cpp-tests/src/rtti/relative.rs) demonstrates
explicit metadata and function-entry decoding without assuming pointer-sized slots.

## Vtable hooking

`cppvtable::hook::VtableHook<'_, I>` installs Rust implementations of virtual methods
on objects of a pointer-layout interface. `iface.hook(mode)`, or
`VtableHook::new(&*iface, mode)`, copies the object's vtable, together with the
ordinary RTTI prefix before its address point, into a heap backup. The prefix is one
pointer for Microsoft tables, two words for Itanium tables, and empty for C and COM
tables. Use `VtableHook::with_prefix(iface, mode, bytes)` for any other prefix, such as
Itanium virtual-base offsets. A Rust `OwnedObject` of a C++ interface has no prefix
unless it was created from an `RttiClass`, so hook it with `with_prefix(.., 0)`.
`HookMode` chooses where replacements go:

- `Shadow` points this object at the copy, so other objects of the class are
  unaffected and `typeid` and `dynamic_cast` keep working through the copied prefix
  (with Clang, casts to `final` classes need `-fno-assume-unique-vtables`; see C++ RTTI).
- `Patch` overwrites the shared table in place, affecting every object that uses it,
  and keeps the copy as a backup. It is not guaranteed to work: compiler vtables
  normally live in read-only memory, making it writable (for every edit and the drop)
  is the caller's job, and native callers may devirtualize the call.

`set(|t| t.method = replacement)` edits a copy of the active table as `I::Vtbl` and
writes back only changed entries. `original()` returns the unhooked `&I::Vtbl` for
forwarding. Index-based `hook(slot, entry)` returns the previous entry, and
`unhook(slot)` puts the original back. Replacements must match each field's
signature and convention (see `#[vtable_fn]`), must not add preconditions to a
method declared as a safe `fn`, and must not unwind. No call through or borrow of the
active table, such as `vtable()`, may overlap an edit. Dropping the hook restores
the object's table pointer (`Shadow`) or every changed entry (`Patch`). Hooks on the
same object or table copy each other's tables, so drop them in reverse installation
order:

```rust,ignore
// SAFETY: `widget` is a live native object with its ordinary RTTI prefix, and
// nothing else touches its vtable until `hook` drops.
let mut hook = unsafe { widget.hook(HookMode::Shadow) };
ORIGINAL.store(hook.original().value as *mut c_void, Ordering::Relaxed);
// SAFETY: `forwarding_value` is a `#[vtable_fn(abi = cpp)]` that calls `ORIGINAL`.
unsafe { hook.set(|t| t.value = forwarding_value) };
```

`RawVtableHook::new(object, prefix_size, entries, mode)` is the untyped form for
tables without a declaration, with the same `hook`/`unhook`/`original(slot)` methods.

See the [hooking fixture](crates/cppvtable-cpp-tests/src/rtti/hook.rs).

## Compiler support and scope

The native compiler test matrix is:

| Platform | Compiler | ABI |
| --- | --- | --- |
| Windows x86 and x86-64 | MSVC | Microsoft C++ |
| Windows x86 and x86-64 | clang-cl | Microsoft C++ |
| Linux x86-64 | clang / clang++ | C / Itanium C++ |

Other architectures are not covered by this matrix. The target determines calling
conventions: Microsoft x86 C++ and Windows GNU x86 C++ use `thiscall`; other
supported C++ targets use the platform C calling convention. COM uses `system`, and
C tables use `C`. Windows GNU retains Itanium aggregate return placement. This
distinction was checked against Clang and Rust-generated IR, with a maintained
cross-compilation check; MinGW is not yet included in the native execution matrix.

This library implements declared virtual interface contracts. It does not generate
arbitrary C++ class layouts, virtual inheritance, constructor/destructor
protocols, covariant-return thunks, or C++ exception interoperability. Use explicit
base interface pointers for foreign multiple inheritance. Rust-created interfaces
require an `RttiClass` for C++ `dynamic_cast` and `typeid`, and must never be passed
to C++ `delete`.

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

With [mise](https://mise.jdx.dev), `mise install` provisions stable Rust, the 1.85.1
MSRV, the cross targets, and cargo-deny. `mise run ci` runs the portable CI checks,
and `mise run x86` runs the Windows i686 MSVC tests and lints; `mise tasks` lists
the individual checks.

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
- `#[interface]` requires `unsafe trait`. Declared `fn` callers are safe; remove the
  `unsafe` blocks around them.
- `I::from_raw_ref(&p)` → `unsafe { I::from_raw(p) }` (returns
  `Option<InterfaceRef>`) or `I::from_non_null(p)`. `cppvtable::InterfaceRef` is now
  `cppvtable_abi::InterfaceRef`, re-exported by all three libraries.
- `vtable()` returns `&Vtbl`: `unsafe { &*x.vtable() }` → `x.vtable()`. `raw_of` and
  `vtable_of` live only at `cppvtable_abi::interface::`.
- `OwnedObject::query_interface` → `try_interface`; `OwnedObject::get` and
  `Object::new` are removed (use `Deref` and `OwnedObject::new`).
  `cppvtable::interface_of` returns an `InterfaceRef`.
- `RttiClass::new(&[..])` → `RttiClass::builder().with::<I>(metadata).build()`;
  `RttiError::InterfaceCount` is removed. `cfg!(target_env = "msvc")` ABI selection →
  `CppAbi::TARGET`; `RttiMetadata::from_interface(abi, p)` → `RttiMetadata::of(&*iface)`.
- The old `VtableHook::new(object, prefix, entries, mode)` is now `RawVtableHook`, and
  `replace`/`restore` are `hook`/`unhook`. `VtableHook<'_, I>` is the typed hook.
- COM: `unsafe impl RefCounted { type Policy = SingleRefCount | DualRefCount; }` →
  `#[implement(..., refcount = single | dual)]`; `ComPtr::cast` → inherent
  `IUnknown::cast`; `ComPtr::from_raw_add_ref(x.as_raw())` → `ComPtr::from_ref(&*x)`;
  `cppvtable_com::OwnedObject` → `ChildObject`; `PrivateRef::from_raw` →
  `PrivateRef::from_raw_add_ref`; out-parameters → `*mut Option<ComPtr<I>>` with
  `write_out`.

## License

MIT.
