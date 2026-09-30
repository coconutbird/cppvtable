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

#![no_std]

pub mod interface;

pub use cppvtable_macro::interface_abi as interface;
pub use interface::{Interface, VtablePtr, raw_of, vtable_of};
