//! ABI-level interface declarations for C and C++ virtual tables.
//!
//! This crate defines the interface pointer and vtable metadata shared by foreign C and
//! C++ callers. COM object lifetime, `IUnknown`, and `QueryInterface` are provided by
//! `cppvtable-com`.

#![no_std]

pub mod interface;

pub use cppvtable_macro::interface_abi as interface;
pub use interface::{Interface, VtablePtr, raw_of, vtable_of};
