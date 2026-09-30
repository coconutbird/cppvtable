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

mod object;

pub use cppvtable_abi::{Interface, VtablePtr, raw_of, vtable_of};
pub use cppvtable_macro::{implement_native as implement, interface_native as interface};
pub use object::{
    CppInterface, Implement, Implements, InterfaceRef, Object, OwnedObject, interface_of,
};
