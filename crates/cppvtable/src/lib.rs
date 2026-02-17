//! C++ VTable interop for Rust (MSVC ABI)
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
//! use vtable::proc::*;
//!
//! #[cpp_interface]
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
//! #[implement(IAnimal)]
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
//! | Slot indices `[N]` | ✅ | ❌ (planned) |
//! | thiscall (x86) | ✅ | ❌ (planned) |
//! | Clean Rust syntax | ❌ | ✅ |
//! | No separate crate | ✅ | N/A |

pub mod decl;

/// Proc-macro approach - re-exports from cppvtable-macro crate
pub mod proc {
    pub use cppvtable_macro::{cpp_interface, implement};
}

// Re-export paste for use by declarative macros
#[doc(hidden)]
pub use paste::paste;

// Re-export common types for macro use
#[doc(hidden)]
pub use std::ffi::c_void;
#[doc(hidden)]
pub use std::sync::atomic::{compiler_fence, Ordering};

