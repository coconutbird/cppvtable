//! The `HRESULT` type and the common COM result codes.
//!
//! An `HRESULT` is a 32-bit signed number. A value that is not negative is a success. A
//! negative value is an error.
//!
//! With the feature `windows-compat` the crate uses `windows_core::HRESULT`. The two
//! types have the same layout (a transparent wrapper of `i32`), the same public field,
//! and the same methods `is_ok` and `is_err`.

#[cfg(feature = "windows-compat")]
pub use windows_core::HRESULT;

#[cfg(not(feature = "windows-compat"))]
pub use own::HRESULT;

#[cfg(not(feature = "windows-compat"))]
mod own {
    use core::fmt;

    /// The result code of a COM method.
    ///
    /// A value that is not negative is a success. A negative value is an error.
    #[repr(transparent)]
    #[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
    #[must_use]
    pub struct HRESULT(pub i32);

    impl HRESULT {
        /// Tell if the code is a success code.
        #[inline]
        #[must_use]
        pub const fn is_ok(self) -> bool {
            self.0 >= 0
        }

        /// Tell if the code is an error code.
        #[inline]
        #[must_use]
        pub const fn is_err(self) -> bool {
            self.0 < 0
        }
    }

    impl fmt::Debug for HRESULT {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "HRESULT(0x{:08X})", self.0)
        }
    }

    impl fmt::Display for HRESULT {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "0x{:08X}", self.0)
        }
    }
}

/// Make an `HRESULT` from the 32-bit code.
///
/// The argument is a `u32` because the header of Windows writes an error code as an
/// unsigned hexadecimal number.
#[inline]
pub const fn hresult(code: u32) -> HRESULT {
    HRESULT(i32::from_ne_bytes(code.to_ne_bytes()))
}

/// The operation was a success.
pub const S_OK: HRESULT = hresult(0x0000_0000);
/// The operation was a success, and the answer is "false".
pub const S_FALSE: HRESULT = hresult(0x0000_0001);
/// The object does not support the interface.
pub const E_NOINTERFACE: HRESULT = hresult(0x8000_4002);
/// A pointer argument is not valid.
pub const E_POINTER: HRESULT = hresult(0x8000_4003);
/// The method is not implemented.
pub const E_NOTIMPL: HRESULT = hresult(0x8000_4001);
/// The operation failed. No better code is applicable.
pub const E_FAIL: HRESULT = hresult(0x8000_4005);
/// The system is out of memory.
pub const E_OUTOFMEMORY: HRESULT = hresult(0x8007_000E);
/// An argument is not valid.
pub const E_INVALIDARG: HRESULT = hresult(0x8007_0057);
/// The call is not permitted at this time.
pub const E_UNEXPECTED: HRESULT = hresult(0x8000_FFFF);

#[cfg(test)]
mod tests {
    use super::{E_NOINTERFACE, S_FALSE, S_OK, hresult};

    #[test]
    fn codes_have_the_correct_sign() {
        assert!(S_OK.is_ok());
        assert!(S_FALSE.is_ok());
        assert!(E_NOINTERFACE.is_err());
        assert_eq!(E_NOINTERFACE.0, -2_147_467_262);
        assert_eq!(hresult(0).0, 0);
    }
}
