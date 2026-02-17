//! COM (Component Object Model) support types
//!
//! This module provides core COM types for use with `#[com_interface]` and `#[com_implement]`.
//!
//! ## Key Types
//! - [`GUID`] - 128-bit globally unique identifier for interfaces
//! - [`HRESULT`] - COM return type for error handling
//! - [`IUnknownVTable`] - Base vtable for all COM interfaces
//!
//! ## Example
//! ```ignore
//! use cppvtable::com::*;
//! use cppvtable::proc::{com_interface, com_implement};
//!
//! #[com_interface("12345678-1234-1234-1234-123456789abc")]
//! pub trait IMyInterface {
//!     fn do_something(&self, x: i32) -> HRESULT;
//! }
//! ```

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

// =============================================================================
// GUID - Globally Unique Identifier
// =============================================================================

/// 128-bit globally unique identifier (GUID/UUID/IID).
///
/// Used for interface identification in COM. Format: `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}`
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl GUID {
    /// Create a new GUID from components
    #[must_use]
    pub const fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Self {
            data1,
            data2,
            data3,
            data4,
        }
    }

    /// The nil/zero GUID
    pub const ZERO: GUID = GUID::new(0, 0, 0, [0; 8]);
}

impl std::fmt::Debug for GUID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            self.data1,
            self.data2,
            self.data3,
            self.data4[0],
            self.data4[1],
            self.data4[2],
            self.data4[3],
            self.data4[4],
            self.data4[5],
            self.data4[6],
            self.data4[7]
        )
    }
}

impl std::fmt::Display for GUID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.data1,
            self.data2,
            self.data3,
            self.data4[0],
            self.data4[1],
            self.data4[2],
            self.data4[3],
            self.data4[4],
            self.data4[5],
            self.data4[6],
            self.data4[7]
        )
    }
}

// =============================================================================
// HRESULT - COM error codes
// =============================================================================

/// COM result type. 0 (S_OK) indicates success, negative values indicate errors.
pub type HRESULT = i32;

/// Success
pub const S_OK: HRESULT = 0;
/// Success, but returned false
pub const S_FALSE: HRESULT = 1;
/// No such interface supported
pub const E_NOINTERFACE: HRESULT = 0x8000_4002_u32 as i32;
/// Invalid pointer
pub const E_POINTER: HRESULT = 0x8000_4003_u32 as i32;
/// Unspecified failure
pub const E_FAIL: HRESULT = 0x8000_4005_u32 as i32;
/// Out of memory
pub const E_OUTOFMEMORY: HRESULT = 0x8007_000E_u32 as i32;
/// Invalid argument
pub const E_INVALIDARG: HRESULT = 0x8007_0057_u32 as i32;
/// Not implemented
pub const E_NOTIMPL: HRESULT = 0x8000_4001_u32 as i32;

/// Check if an HRESULT indicates success (non-negative)
#[inline]
#[must_use]
pub const fn succeeded(hr: HRESULT) -> bool {
    hr >= 0
}

/// Check if an HRESULT indicates failure (negative)
#[inline]
#[must_use]
pub const fn failed(hr: HRESULT) -> bool {
    hr < 0
}

// =============================================================================
// IUnknown - Base COM interface
// =============================================================================

/// IUnknown interface ID
pub const IID_IUNKNOWN: GUID = GUID::new(
    0x00000000,
    0x0000,
    0x0000,
    [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
);

/// IUnknown - base of all COM interfaces.
///
/// Every COM interface vtable starts with these three methods at slots 0, 1, 2.
/// This generates:
/// - `IUnknownVTable` struct with function pointers
/// - `IUnknown` wrapper struct with safe methods
/// - `VTableLayout` impl
/// - `iunknown_forwarders!` macro for derived interfaces
/// - `iunknown_base_vtable!` macro for vtable initialization
#[crate::proc::cppvtable(stdcall, no_iid, internal)]
pub trait IUnknown {
    /// Query for another interface by GUID.
    fn query_interface(&self, riid: *const GUID, ppv: *mut *mut c_void) -> HRESULT;

    /// Increment reference count. Returns new count.
    fn add_ref(&self) -> u32;

    /// Decrement reference count. Returns new count.
    fn release(&mut self) -> u32;
}

// =============================================================================
// ComRefCount - Atomic reference counter for COM objects
// =============================================================================

/// Atomic reference counter for COM objects.
///
/// Embed this in your COM object struct for automatic reference counting.
/// Use with `#[com_implement]` for auto-generated AddRef/Release.
#[repr(transparent)]
pub struct ComRefCount(AtomicU32);

impl ComRefCount {
    /// Create a new reference counter with count = 1
    #[must_use]
    pub const fn new() -> Self {
        Self(AtomicU32::new(1))
    }

    /// Increment the reference count. Returns the new count.
    #[inline]
    pub fn add_ref(&self) -> u32 {
        self.0.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Decrement the reference count. Returns the new count.
    ///
    /// When count reaches 0, the caller should destroy the object.
    #[inline]
    pub fn release(&self) -> u32 {
        self.0.fetch_sub(1, Ordering::Release) - 1
    }

    /// Get the current reference count.
    #[inline]
    #[must_use]
    pub fn count(&self) -> u32 {
        self.0.load(Ordering::Relaxed)
    }
}

impl Default for ComRefCount {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Helper trait for COM interface identification
// =============================================================================

/// Trait for types that have a COM interface ID (IID).
///
/// Automatically implemented by `#[com_interface]`.
pub trait ComInterface {
    /// The interface ID (IID) for this interface.
    const IID: GUID;
}

// =============================================================================
// IUnknown method implementations macro
// =============================================================================

/// Generates the IUnknown method implementations for a COM object.
///
/// Expects the struct to have a `ref_count: ComRefCount` field.
#[macro_export]
macro_rules! iunknown_methods {
    ($struct_type:ty, $vtable_field:ident, $iid_const:ident) => {
        /// Query for another interface by GUID.
        ///
        /// Returns `S_OK` if the interface is supported, `E_NOINTERFACE` otherwise.
        ///
        /// # Safety
        /// - `riid` must point to a valid GUID
        /// - `ppv` must point to a valid, writable pointer location
        pub unsafe fn query_interface(
            &self,
            riid: *const $crate::GUID,
            ppv: *mut *mut ::std::ffi::c_void,
        ) -> $crate::HRESULT {
            unsafe {
                if ppv.is_null() {
                    return $crate::E_POINTER;
                }

                let riid_ref = &*riid;

                // Check if requested IID matches this interface or IUnknown
                if *riid_ref == $iid_const || *riid_ref == $crate::IID_IUNKNOWN {
                    let ptr = &self.$vtable_field as *const _ as *mut ::std::ffi::c_void;
                    *ppv = ptr;
                    self.add_ref();
                    return $crate::S_OK;
                }

                *ppv = ::std::ptr::null_mut();
                $crate::E_NOINTERFACE
            }
        }

        /// Increment the reference count.
        pub fn add_ref(&self) -> u32 {
            self.ref_count.add_ref()
        }

        /// Decrement the reference count.
        pub fn release(&mut self) -> u32 {
            self.ref_count.release()
        }
    };
}
