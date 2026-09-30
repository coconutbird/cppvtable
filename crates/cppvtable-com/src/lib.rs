//! COM object ownership and reference-counting runtime built on `cppvtable-abi`.
//!
//! Declare and implement COM interfaces directly through this crate. Interfaces use
//! the shared ABI interface metadata, while
//! `IUnknown`, `QueryInterface`, owning pointers, and reference-count policies live here.

pub mod guid;
pub mod hresult;
pub mod interface;
pub mod object;
pub mod ptr;
pub mod refcount;

pub use cppvtable_abi::{Interface, VtablePtr, raw_of, vtable_of};
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
