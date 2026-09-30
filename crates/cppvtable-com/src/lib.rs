//! COM object ownership and reference-counting runtime built on `cppvtable-abi`.
//!
//! Declare and implement COM interfaces directly through this crate. Interfaces use
//! the shared ABI interface metadata, while
//! `IUnknown`, `QueryInterface`, owning pointers, and reference-count policies live here.
//!
//! A declaration's method safety is preserved in its generated implementation trait.
//! Ordinary Rust methods can be called directly on a standalone implementation value:
//!
//! ```
//! use cppvtable_com::{RefCounted, SingleRefCount, implement, interface};
//! #[interface(abi = com, iid = "c0640001-0000-4000-8000-000000000001")]
//! unsafe trait IValue {
//!     fn Value(&self) -> u32;
//!     /// # Safety
//!     /// `output` must be aligned and writable for one `u32`.
//!     unsafe fn Write(&self, output: *mut u32);
//! }
//! #[implement(IValue)]
//! struct Value;
//! // SAFETY: Default standalone policy hooks return no auxiliary pointers.
//! unsafe impl RefCounted for Value { type Policy = SingleRefCount; }
//! impl IValueImpl for Value {
//!     fn Value(&self) -> u32 { 42 }
//!     unsafe fn Write(&self, output: *mut u32) {
//!         // SAFETY: The method's caller supplies writable output.
//!         unsafe { *output = self.Value() };
//!     }
//! }
//! assert_eq!(Value.Value(), 42);
//! let mut output = 0;
//! // SAFETY: The output is a live, aligned local variable.
//! unsafe { Value.Write(&raw mut output) };
//! assert_eq!(output, 42);
//! ```
//!
//! Methods with pointer or allocation preconditions must be declared `unsafe fn`.
//! Direct implementation calls require `unsafe` too:
//!
//! ```compile_fail,E0133
//! use cppvtable_com::{RefCounted, SingleRefCount, implement, interface};
//! #[interface(abi = com, iid = "c0640002-0000-4000-8000-000000000002")]
//! unsafe trait IWrite {
//!     /// # Safety
//!     /// `output` must be aligned and writable for one `u32`.
//!     unsafe fn Write(&self, output: *mut u32);
//! }
//! #[implement(IWrite)]
//! struct Writer;
//! // SAFETY: Default standalone policy hooks return no auxiliary pointers.
//! unsafe impl RefCounted for Writer { type Policy = SingleRefCount; }
//! impl IWriteImpl for Writer {
//!     unsafe fn Write(&self, output: *mut u32) {
//!         // SAFETY: The method's caller supplies writable output.
//!         unsafe { *output = 42 };
//!     }
//! }
//! let mut output = 0;
//! Writer.Write(&raw mut output); // An unsafe implementation call requires `unsafe`.
//! ```

#![no_std]

extern crate alloc;

pub mod guid;
pub mod hresult;
pub mod interface;
pub mod object;
pub mod ptr;
pub mod refcount;

pub use cppvtable_abi::{Interface, VtableLayout, VtablePtr, raw_of, vtable_of};
pub use cppvtable_macro::{implement, interface};
pub use guid::GUID;

pub use hresult::{
    E_FAIL, E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED,
    HRESULT, S_FALSE, S_OK, hresult,
};
pub use interface::{
    AgileInterface, ComInterface, IUnknown, IUnknownVtbl, interface_matches, unknown_add_ref,
    unknown_release,
};
pub use object::{ComImplement, ComObject, Implements, OwnedObject, interface_of, query_interface};
pub use ptr::{ComPtr, PrivateRef, object_of_raw};
pub use refcount::{
    DualRefCount, DualState, ForwardRefCount, ForwardState, PrivatePolicy, RefCountPolicy,
    RefCounted, SingleRefCount, SingleState, StandalonePolicy,
};
