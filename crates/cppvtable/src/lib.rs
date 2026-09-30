//! The ABI layer: COM and C++ vtables, the object model, and the reference count
//! policies.
//!
//! A frontend implements binary interfaces of another language. The interfaces use
//! different calling conventions on different targets. This crate owns all of that, so
//! a frontend has no hand-written vtable and no hand-written shim.
//!
//! The crate has no dependency on `RenderBridge`.
//!
//! # Declare an interface
//!
//! ```
//! use core::ffi::c_void;
//! use cppvtable::{HRESULT, interface};
//!
//! #[interface(abi = com, iid = "1c1a0b4f-2a4a-4a1b-9a4a-0f0a0b0c0d01")]
//! pub unsafe trait IThing {
//!     /// Give the value of the thing.
//!     fn GetValue(&self, value: *mut u32) -> HRESULT;
//! }
//! ```
//!
//! The macro makes:
//!
//! | Item | Function |
//! | ---- | -------- |
//! | `IThing` | the interface type. A transparent wrapper of one interface pointer. |
//! | `IThingVtbl` | the `#[repr(C)]` vtable. The first field is the vtable of the base. |
//! | `impl Interface for IThing` | `type Vtbl`, `const IID`, `const ANCESTORS`. |
//! | `impl Deref for IThing` | the base interface, so a `ComPtr` gives the whole chain. |
//! | `IThingImpl` | the trait of the implementer. Each method takes `&self`. |
//! | `IThingVtbl::new::<T, SLOT>()` | the builder of a static vtable. |
//!
//! # Implement an object
//!
//! ```
//! # use core::ffi::c_void;
//! # use core::sync::atomic::{AtomicU32, Ordering};
//! # use cppvtable::{HRESULT, S_OK, interface};
//! # #[interface(abi = com, iid = "1c1a0b4f-2a4a-4a1b-9a4a-0f0a0b0c0d02")]
//! # pub unsafe trait IThing {
//! #     fn GetValue(&self, value: *mut u32) -> HRESULT;
//! # }
//! use cppvtable::{ComObject, RefCounted, SingleRefCount, implement};
//!
//! #[implement(IThing)]
//! pub struct Thing {
//!     value: AtomicU32,
//! }
//!
//! impl RefCounted for Thing {
//!     type Policy = SingleRefCount;
//! }
//!
//! impl IThingImpl for Thing {
//!     fn GetValue(&self, value: *mut u32) -> HRESULT {
//!         // SAFETY: The caller of the COM method gives a writable place.
//!         unsafe { *value = self.value.load(Ordering::Relaxed) };
//!         S_OK
//!     }
//! }
//!
//! let thing = ComObject::new(Thing { value: AtomicU32::new(7) });
//! let mut out = 0_u32;
//! // SAFETY: The pointer refers to a local value.
//! let result = unsafe { thing.GetValue(&raw mut out) };
//! assert!(result.is_ok());
//! assert_eq!(out, 7);
//! ```
//!
//! # The object model
//!
//! [`ComObject<T>`] is the allocation: one vtable pointer for each implemented
//! interface chain, then the counts, then the Rust value. See the module
//! [`object`](crate::object) for the layout and for the `this` adjustment.
//!
//! [`ComPtr<I>`] owns one public reference. [`PrivateRef<T>`] owns one private
//! reference. [`OwnedObject<T>`] owns a child object that a container destroys.
//!
//! # The reference counts
//!
//! See the module [`refcount`](crate::refcount) for the rules, the hooks, and a
//! Direct3D 9 example.
//!
//! # The feature `windows-compat`
//!
//! With this feature [`GUID`] and [`HRESULT`] are the types of `windows-core`. The two
//! versions have the same layout and the same constructors, so the rest of the crate
//! does not change.

pub mod guid;
pub mod hresult;
pub mod interface;
pub mod object;
pub mod ptr;
pub mod refcount;

pub use cppvtable_macro::{implement, interface};

pub use guid::GUID;
pub use hresult::{
    E_FAIL, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED,
    HRESULT, S_FALSE, S_OK, hresult,
};
pub use interface::{
    IUnknown, IUnknownVtbl, Interface, VtablePtr, interface_matches, raw_of, unknown_add_ref,
    unknown_release, vtable_of,
};
pub use object::{ComImplement, ComObject, Implements, OwnedObject, interface_of, query_interface};
pub use ptr::{ComPtr, PrivateRef, object_of_raw};
pub use refcount::{
    DualRefCount, DualState, ForwardRefCount, ForwardState, PrivatePolicy, RefCountPolicy,
    RefCounted, SingleRefCount, SingleState, StandalonePolicy,
};
