//! COM object ownership and reference-counting runtime built on `cppvtable-abi`.
//!
//! Declare and implement COM interfaces directly through this crate. Interfaces use
//! the shared ABI interface metadata, while
//! `IUnknown`, `QueryInterface`, owning pointers, and reference-count policies live here.
//!
//! This is a `no_std` library using `alloc` for object storage. Applications must
//! provide a global allocator when constructing objects; reference-count policies
//! require target support for 32-bit atomics. `windows-compat` substitutes Windows
//! types only on Windows targets and keeps the local representations elsewhere.
//!
//! A declaration's method safety is preserved in the generated callers and in its
//! implementation trait. A method declared `fn` is safe to call, both through an
//! interface pointer and on a standalone implementation value:
//!
//! ```
//! use cppvtable_com::{ComObject, implement, interface};
//! #[interface(abi = com, iid = "c0640001-0000-4000-8000-000000000001")]
//! unsafe trait IValue {
//!     fn Value(&self) -> u32;
//!     /// # Safety
//!     /// `output` must be aligned and writable for one `u32`.
//!     unsafe fn Write(&self, output: *mut u32);
//! }
//! #[implement(IValue, refcount = single)]
//! struct Value;
//! impl IValueImpl for Value {
//!     fn Value(&self) -> u32 { 42 }
//!     unsafe fn Write(&self, output: *mut u32) {
//!         // SAFETY: The method's caller supplies writable output.
//!         unsafe { *output = self.Value() };
//!     }
//! }
//! assert_eq!(Value.Value(), 42);
//! let pointer = ComObject::new(Value);
//! assert_eq!(pointer.Value(), 42);
//! let mut output = 0;
//! // SAFETY: The output is a live, aligned local variable.
//! unsafe { pointer.Write(&raw mut output) };
//! assert_eq!(output, 42);
//! ```
//!
//! Methods with pointer or allocation preconditions must be declared `unsafe fn`.
//! Direct implementation calls require `unsafe` too:
//!
//! ```compile_fail,E0133
//! use cppvtable_com::{implement, interface};
//! #[interface(abi = com, iid = "c0640002-0000-4000-8000-000000000002")]
//! unsafe trait IWrite {
//!     /// # Safety
//!     /// `output` must be aligned and writable for one `u32`.
//!     unsafe fn Write(&self, output: *mut u32);
//! }
//! #[implement(IWrite, refcount = single)]
//! struct Writer;
//! impl IWriteImpl for Writer {
//!     unsafe fn Write(&self, output: *mut u32) {
//!         // SAFETY: The method's caller supplies writable output.
//!         unsafe { *output = 42 };
//!     }
//! }
//! let mut output = 0;
//! Writer.Write(&raw mut output); // An unsafe implementation call requires `unsafe`.
//! ```
//!
//! # Reference counts
//!
//! `#[implement(IThing, refcount = single)]` and `refcount = dual` select
//! [`SingleRefCount`] or [`DualRefCount`] with the default hooks. Implement
//! [`RefCounted`] by hand for [`ForwardRefCount`] children, owned by a
//! [`ChildObject`], and for policies with hooks. See [`refcount`].
//!
//! An object behind an [`AgileInterface`] may be used and destroyed on any thread, so
//! the shorthand refuses a type that is not `Send + Sync`:
//!
//! ```compile_fail,E0277
//! use std::rc::Rc;
//! use cppvtable_com::{AgileInterface, implement, interface};
//! #[interface(abi = com, iid = "c0640005-0000-4000-8000-000000000005")]
//! unsafe trait IAgile {
//!     fn Value(&self) -> u32;
//! }
//! // SAFETY: Implementations of `IAgile` support use and destruction on any thread.
//! unsafe impl AgileInterface for IAgile {}
//! #[implement(IAgile, refcount = single)]
//! struct Local {
//!     value: Rc<u32>, // `Rc` is neither `Send` nor `Sync`.
//! }
//! impl IAgileImpl for Local {
//!     fn Value(&self) -> u32 { *self.value }
//! }
//! ```
//!
//! # Out-parameters
//!
//! Declare an interface out-parameter as `*mut Option<ComPtr<I>>`. `Option<ComPtr<I>>`
//! has the layout of one nullable interface pointer, so the parameter has the ABI of
//! `I** out` in C and C++. An implementation fills it with [`write_out`], which never
//! drops the old contents of a slot that a foreign caller may leave uninitialized. A
//! Rust caller passes a local `None` and owns the answer, with no raw pointer:
//!
//! ```
//! use cppvtable_com::{ComObject, ComPtr, HRESULT, implement, interface, write_out};
//! #[interface(abi = com, iid = "c0640003-0000-4000-8000-000000000003")]
//! unsafe trait ICount {
//!     fn Count(&self) -> u32;
//! }
//! #[interface(abi = com, iid = "c0640004-0000-4000-8000-000000000004")]
//! unsafe trait IFactory {
//!     /// # Safety
//!     /// `out` must be null or aligned and writable for one interface pointer.
//!     unsafe fn Create(&self, count: u32, out: *mut Option<ComPtr<ICount>>) -> HRESULT;
//! }
//! #[implement(ICount, refcount = single)]
//! struct Count {
//!     count: u32,
//! }
//! impl ICountImpl for Count {
//!     fn Count(&self) -> u32 { self.count }
//! }
//! #[implement(IFactory, refcount = single)]
//! struct Factory;
//! impl IFactoryImpl for Factory {
//!     unsafe fn Create(&self, count: u32, out: *mut Option<ComPtr<ICount>>) -> HRESULT {
//!         // SAFETY: The method's caller supplies a null or writable slot.
//!         unsafe { write_out(out, Some(ComObject::new(Count { count }))) }
//!     }
//! }
//! let factory = ComObject::new(Factory);
//! let mut created = None;
//! // SAFETY: `created` is a live local slot.
//! assert!(unsafe { factory.Create(7, &raw mut created) }.is_ok());
//! assert_eq!(created.unwrap().Count(), 7);
//! ```
//!
//! A borrowed interface asks for another interface with [`IUnknown::cast`], and
//! [`ComPtr::from_ref`] turns a borrow into an owning reference.
//!
//! An interface value exists only behind a reference or an owner, such as [`ComPtr`]
//! or [`InterfaceRef`], that keeps the object alive. The newtype is neither `Copy` nor
//! `Clone`, so it cannot outlive that owner:
//!
//! ```compile_fail,E0507
//! use cppvtable_com::{ComObject, implement, interface};
//! #[interface(abi = com, iid = "c0640006-0000-4000-8000-000000000006")]
//! unsafe trait IValue {
//!     fn Value(&self) -> u32;
//! }
//! #[implement(IValue, refcount = single)]
//! struct Value;
//! impl IValueImpl for Value {
//!     fn Value(&self) -> u32 { 42 }
//! }
//! let pointer = ComObject::new(Value);
//! let copy: IValue = *pointer; // Moving the interface out of the borrow is refused.
//! drop(pointer);
//! copy.Value();
//! ```

#![no_std]

extern crate alloc;

pub mod guid;
pub mod hresult;
pub mod interface;
pub mod object;
pub mod ptr;
pub mod refcount;

#[doc(hidden)]
pub use cppvtable_abi::interface::RawInterface;
pub use cppvtable_abi::{Interface, InterfaceRef, VtableLayout, VtablePtr, vtable_fn};
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
pub use object::{ChildObject, ComImplement, ComObject, Implements, interface_of, query_interface};
pub use ptr::{ComPtr, PrivateRef, object_of_raw, write_out};
pub use refcount::{
    DualRefCount, DualState, ForwardRefCount, ForwardState, PrivatePolicy, RefCountPolicy,
    RefCounted, SingleRefCount, SingleState, StandalonePolicy,
};
