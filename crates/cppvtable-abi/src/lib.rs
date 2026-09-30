//! ABI-level interface declarations for C and C++ virtual tables.
//!
//! This crate defines the interface pointer and vtable metadata shared by foreign C and
//! C++ callers. COM object lifetime, `IUnknown`, and `QueryInterface` are provided by
//! `cppvtable-com`.
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

pub use cppvtable_macro::interface_abi as interface;
pub use interface::{Interface, VtableLayout, VtablePtr, raw_of, vtable_of};
