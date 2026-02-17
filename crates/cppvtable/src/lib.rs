//! C++ VTable interop for Rust (MSVC ABI)
//!
//! This crate provides C++ compatible vtable layouts with optional RTTI support.
//!
//! ## RTTI (Runtime Type Information)
//!
//! When enabled, vtables include type information at slot -1 (negative offset),
//! matching the MSVC and Itanium ABIs. This enables:
//! - Runtime type identification
//! - Safe cross-casting between interfaces (like `dynamic_cast`)
//!
//! This crate provides two approaches for defining C++ compatible interfaces:
//!
//! ## Declarative macros (`decl` module)
//! ```ignore
//! use vtable::decl::*;
//!
//! define_interface! {
//!     interface IAnimal {
//!         fn speak(&self);
//!         [5] fn legs(&self) -> i32;  // explicit slot index
//!     }
//! }
//!
//! define_class! {
//!     class Dog : IAnimal {
//!         name: [u8; 32],
//!     }
//! }
//! ```
//!
//! ## Proc-macros (`proc` module)
//! ```ignore
//! use cppvtable::proc::*;
//!
//! #[cppvtable]
//! pub trait IAnimal {
//!     fn speak(&self);
//!     fn legs(&self) -> i32;
//! }
//!
//! #[repr(C)]
//! pub struct Dog {
//!     vtable: *const IAnimalVTable,
//!     name: [u8; 32],
//! }
//!
//! #[cppvtable_impl(IAnimal)]
//! impl Dog {
//!     fn speak(&self) { println!("Woof!"); }
//!     fn legs(&self) -> i32 { 4 }
//! }
//! ```
//!
//! ## Feature comparison
//!
//! | Feature | Declarative | Proc-macro |
//! |---------|-------------|------------|
//! | Slot indices `[N]` / `#[slot(N)]` | ✅ | ✅ |
//! | thiscall (x86) | ✅ | ✅ |
//! | Clean Rust syntax | ❌ | ✅ |
//! | No separate crate | ✅ | N/A |
//! | RTTI support | ✅ | ✅ |
//! | Multiple inheritance | ✅ | ✅ |

pub mod com;
pub mod decl;
pub mod rtti;

// =============================================================================
// VTableLayout - Trait for interface inheritance
// =============================================================================

/// Trait providing vtable layout information for interface inheritance.
///
/// This trait is automatically implemented by `#[cppvtable]` for each interface.
/// It enables `extends(Base)` to inherit from another interface.
///
/// # Example
/// ```ignore
/// use cppvtable::proc::cppvtable;
///
/// #[cppvtable]
/// pub trait IBase {
///     fn base_method(&self);
/// }
///
/// #[cppvtable(extends(IBase))]
/// pub trait IDerived {
///     fn derived_method(&self);  // Starts at slot 1
/// }
/// ```
pub trait VTableLayout {
    /// The number of vtable slots used by this interface (including inherited slots).
    const SLOT_COUNT: usize;

    /// The vtable struct type for this interface.
    type VTable;
}

/// Proc-macro approach - re-exports from cppvtable-macro crate
pub mod proc {
    pub use cppvtable_macro::{com_implement, com_interface};
    pub use cppvtable_macro::{cppvtable, cppvtable_impl};
}

// Re-export paste for use by declarative macros
#[doc(hidden)]
pub use paste::paste;

// Re-export common types for macro use
#[doc(hidden)]
pub use std::ffi::c_void;
#[doc(hidden)]
pub use std::sync::atomic::{Ordering, compiler_fence};

// Re-export RTTI types for macro-generated code
#[doc(hidden)]
pub use rtti::{InterfaceInfo, TypeInfo};

// Re-export COM types for macro-generated code
#[doc(hidden)]
pub use com::{
    ComRefCount, E_NOINTERFACE, E_POINTER, GUID, HRESULT, IID_IUNKNOWN, IUnknown, IUnknownVTable,
    S_OK,
};
