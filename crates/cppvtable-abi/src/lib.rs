//! ABI-level interface declarations for C and C++ virtual tables.
//!
//! This crate defines the interface pointer and vtable metadata shared by foreign C and
//! C++ callers. COM object lifetime, `IUnknown`, and `QueryInterface` are provided by
//! `cppvtable-com`.
//!
//! A declaration is an `unsafe trait`. The `unsafe` is the declarer's promise that the
//! slot order, signatures, calling conventions, and return lowering match the foreign
//! header, that every method declared as a safe `fn` has no precondition beyond a live
//! object, and that no method unwinds. Borrowing a foreign pointer is the single unsafe
//! step; safe methods are then called without `unsafe`:
//!
//! ```
//! use core::ffi::c_void;
//!
//! #[cppvtable_abi::interface(abi = c)]
//! pub unsafe trait ICounter {
//!     fn value(&self) -> u32;
//! }
//!
//! fn read(raw: *mut c_void) -> Option<u32> {
//!     // SAFETY: The caller gives a null pointer or a live `ICounter` object.
//!     let counter = unsafe { ICounter::from_raw(raw) }?;
//!     Some(counter.value())
//! }
//! # assert_eq!(read(core::ptr::null_mut()), None);
//! ```
//!
//! A declaration without `unsafe` is rejected:
//!
//! ```compile_fail
//! #[cppvtable_abi::interface(abi = c)]
//! pub trait ICounter {
//!     fn value(&self) -> u32;
//! }
//! ```
//!
//! An interface value cannot be copied out of its borrow:
//!
//! ```compile_fail,E0507
//! #[cppvtable_abi::interface(abi = c)]
//! pub unsafe trait ICounter {
//!     fn value(&self) -> u32;
//! }
//!
//! fn escape(counter: cppvtable_abi::InterfaceRef<'_, ICounter>) -> ICounter {
//!     *counter
//! }
//! ```
//!
//! `#[vtable_fn(abi = ...)]` gives a free function the exact calling convention of the
//! vtable fields of that ABI on every target, so it can be stored in a hand-built table:
//!
//! ```
//! use core::ffi::c_void;
//!
//! #[cppvtable_abi::interface(abi = cpp)]
//! pub unsafe trait ICounter {
//!     fn value(&self) -> u32;
//! }
//!
//! #[cppvtable_abi::vtable_fn(abi = cpp)]
//! unsafe fn value(_this: *mut c_void) -> u32 {
//!     7
//! }
//!
//! let table = ICounterVtbl { value };
//! # let _ = table;
//! ```
//!
//! `slots` declares the total vtable extent, including inherited and reserved entries.
//! The extent cannot end before a known method:
//!
//! ```compile_fail,E0080
//! #[cppvtable_abi::interface(abi = c, slots = 3)]
//! unsafe trait IPartial {
//!     #[slot(3)]
//!     fn known(&self) -> u32;
//! }
//! ```
//!
//! A derived extent must also include the full base vtable:
//!
//! ```compile_fail,E0080
//! #[cppvtable_abi::interface(abi = c, slots = 50)]
//! unsafe trait IBase {
//!     #[slot(32)]
//!     fn known(&self) -> u32;
//! }
//! #[cppvtable_abi::interface(abi = c, extends(IBase), slots = 50)]
//! unsafe trait IDerived {
//!     fn beyond_base(&self) -> u32;
//! }
//! ```
//!
//! Inline callback tables are C headers, so C++ ABI declarations reject them:
//!
//! ```compile_fail
//! #[cppvtable_abi::interface(abi = cpp, layout = inline)]
//! unsafe trait ICppInline { fn value(&self) -> u32; }
//! ```
//!
//! A base and its derived interface must use the same table representation. A
//! pointer-layout base cannot become an inline prefix:
//!
//! ```compile_fail,E0080
//! #[cppvtable_abi::interface(abi = c)]
//! unsafe trait IPointer { fn value(&self) -> u32; }
//! #[cppvtable_abi::interface(abi = c, layout = inline, extends(IPointer))]
//! unsafe trait IInlineDerived { fn extra(&self) -> u32; }
//! ```
//!
//! An inline base cannot become a pointer-layout prefix either:
//!
//! ```compile_fail,E0080
//! #[cppvtable_abi::interface(abi = c, layout = inline)]
//! unsafe trait IInline { fn value(&self) -> u32; }
//! #[cppvtable_abi::interface(abi = c, extends(IInline))]
//! unsafe trait IPointerDerived { fn extra(&self) -> u32; }
//! ```
//!
//! An inline header must contain at least one entry:
//!
//! ```compile_fail,E0080
//! #[cppvtable_abi::interface(abi = c, layout = inline)]
//! unsafe trait IEmptyInline {}
//! ```

#![no_std]

pub mod interface;
pub mod rtti;

pub use cppvtable_macro::{interface_abi as interface, vtable_fn};
pub use interface::{Interface, InterfaceRef, RawInterface, VtableLayout, VtablePtr};
