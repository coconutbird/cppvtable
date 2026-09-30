//! Callable C/C++ interfaces and Rust implementations of foreign object layouts.
//!
//! [`interface`] declares both a borrowed caller interface and its implementation
//! trait. [`implement`] builds static vtables for a Rust type. [`OwnedObject`] keeps
//! the resulting object at a stable address and controls its lifetime.
//!
//! COM interfaces and their reference counting live in the separate `cppvtable-com`
//! crate.
//!
//! A borrowed interface cannot outlive its owner:
//!
//! ```compile_fail
//! use cppvtable::{implement, interface, OwnedObject};
//! #[interface(abi = c)]
//! unsafe trait IValue { fn value(&self) -> u32; }
//! #[implement(IValue)]
//! struct Value;
//! impl IValueImpl for Value { fn value(&self) -> u32 { 7 } }
//! let view = {
//!     let owner = OwnedObject::new(Value);
//!     owner.interface::<IValue>()
//! };
//! unsafe { view.value(); }
//! ```
//!
//! COM declarations use the separate crate:
//!
//! ```compile_fail
//! #[cppvtable::interface(abi = com, iid = "00000000-0000-0000-C000-000000000046")]
//! unsafe trait ICom { fn value(&self) -> u32; }
//! ```

//! Implementation methods preserve each declaration's method safety. Safe methods
//! must support direct calls on standalone Rust values and every argument allowed by
//! their Rust signatures. Declare `unsafe fn` and document the contract for methods
//! requiring valid foreign pointers or an allocation-embedded `self`. Generated shims
//! recover an embedded `self` before calling implementation methods. Thread access
//! follows the interface and owner's contract; a shared reference alone does not
//! authorize concurrent foreign calls.
//!
//! Calling an unsafe implementation method directly still requires `unsafe`:
//!
//! ```compile_fail,E0133
//! use cppvtable::{implement, interface};
//! #[interface(abi = c)]
//! unsafe trait IReader {
//!     /// # Safety
//!     /// `input` must point to a live readable u32.
//!     unsafe fn read(&self, input: *const u32) -> u32;
//! }
//! #[implement(IReader)]
//! struct Reader;
//! impl IReaderImpl for Reader {
//!     unsafe fn read(&self, input: *const u32) -> u32 { unsafe { *input } }
//! }
//! let input = 7;
//! IReaderImpl::read(&Reader, &input);
//! ```
//!
//! Declared unsafety is preserved even for scalar methods without pointer parameters:
//!
//! ```compile_fail,E0133
//! use cppvtable::{implement, interface};
//! #[interface(abi = c)]
//! unsafe trait IProtocol {
//!     /// # Safety
//!     /// The caller must establish the application's ready state.
//!     unsafe fn ready_value(&self) -> u32;
//! }
//! #[implement(IProtocol)]
//! struct Protocol;
//! impl IProtocolImpl for Protocol { unsafe fn ready_value(&self) -> u32 { 7 } }
//! IProtocolImpl::ready_value(&Protocol);
//! ```
//!
//! A valid direct call can establish the documented argument precondition:
//!
//! ```
//! use cppvtable::{implement, interface};
//! #[interface(abi = c)]
//! unsafe trait IReader {
//!     fn value(&self) -> u32;
//!     fn passthrough(&self, pointer: *const u32) -> *const u32;
//!     /// # Safety
//!     /// `input` must point to a live readable u32.
//!     unsafe fn read(&self, input: *const u32) -> u32;
//! }
//! #[implement(IReader)]
//! struct Reader;
//! impl IReaderImpl for Reader {
//!     fn value(&self) -> u32 { 5 }
//!     fn passthrough(&self, pointer: *const u32) -> *const u32 { pointer }
//!     unsafe fn read(&self, input: *const u32) -> u32 { unsafe { *input } }
//! }
//! let reader = Reader;
//! let input = IReaderImpl::value(&reader);
//! assert_eq!(IReaderImpl::passthrough(&reader, &input), &input as *const u32);
//! // SAFETY: `input` remains alive and readable throughout the call.
//! assert_eq!(unsafe { IReaderImpl::read(&reader, &input) }, 5);
//! ```

mod object;

pub use cppvtable_abi::{Interface, VtablePtr, raw_of, vtable_of};
pub use cppvtable_macro::{implement_native as implement, interface_native as interface};
pub use object::{
    CppInterface, Implement, Implements, InterfaceRef, Object, OwnedObject, interface_of,
};
