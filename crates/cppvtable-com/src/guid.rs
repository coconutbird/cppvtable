//! The ABI representation of a 128-bit interface or class identifier.
//!
//! COM uses this layout for IIDs and CLSIDs.
//! On Windows with `windows-compat`, this name is `windows_core::GUID`. Other targets
//! keep the local representation even when that feature is enabled.

#[cfg(all(windows, feature = "windows-compat"))]
pub use windows_core::GUID;

#[cfg(not(all(windows, feature = "windows-compat")))]
pub use own::GUID;

#[cfg(not(all(windows, feature = "windows-compat")))]
mod own {
    use core::fmt;

    /// A 128-bit identifier with the layout of the Win32 `GUID` structure.
    #[repr(C)]
    #[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
    pub struct GUID {
        /// The first 8 hexadecimal digits.
        pub data1: u32,
        /// The first group of 4 hexadecimal digits.
        pub data2: u16,
        /// The second group of 4 hexadecimal digits.
        pub data3: u16,
        /// The last 2 groups: 2 bytes and then 6 bytes.
        pub data4: [u8; 8],
    }

    impl GUID {
        /// Make a `GUID` from the four fields.
        #[inline]
        #[must_use]
        pub const fn from_values(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
            Self {
                data1,
                data2,
                data3,
                data4,
            }
        }

        /// Make a `GUID` from a `u128`. The `u128` holds the digits in text order.
        #[inline]
        #[must_use]
        pub const fn from_u128(value: u128) -> Self {
            let b = value.to_be_bytes();
            Self {
                data1: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
                data2: u16::from_be_bytes([b[4], b[5]]),
                data3: u16::from_be_bytes([b[6], b[7]]),
                data4: [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
            }
        }

        /// Give the `GUID` as a `u128` with the digits in text order.
        #[inline]
        #[must_use]
        pub const fn to_u128(&self) -> u128 {
            let a = self.data1.to_be_bytes();
            let b = self.data2.to_be_bytes();
            let c = self.data3.to_be_bytes();
            let d = self.data4;
            u128::from_be_bytes([
                a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5],
                d[6], d[7],
            ])
        }

        /// Give the `GUID` that has all bytes zero.
        #[inline]
        #[must_use]
        pub const fn zeroed() -> Self {
            Self::from_values(0, 0, 0, [0; 8])
        }
    }

    impl fmt::Debug for GUID {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{{{self}}}")
        }
    }

    impl fmt::Display for GUID {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
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
}

#[cfg(test)]
mod tests {
    use super::GUID;

    #[test]
    fn u128_round_trip() {
        let id = GUID::from_values(
            0xd022_3b96,
            0xbf7a,
            0x43fd,
            [0x92, 0xbd, 0xa4, 0x3b, 0x0d, 0x82, 0xb9, 0xeb],
        );
        assert_eq!(GUID::from_u128(id.to_u128()), id);
        assert_eq!(id.to_u128(), 0xd022_3b96_bf7a_43fd_92bd_a43b_0d82_b9eb);
    }
}
