//! Direct ABI declarations and borrowed calls without a COM dependency.

use core::ffi::c_void;
use core::mem::{offset_of, size_of};

use cppvtable_abi::{Interface, interface, raw_of, vtable_of};

/// The base C interface.
#[interface(abi = c)]
pub unsafe trait IValue {
    /// Read the value.
    fn Get(&self) -> u32;
}

/// A C interface with the base table as its prefix.
#[interface(abi = c, extends(IValue))]
pub unsafe trait IValueWithOffset {
    /// Read the value plus an offset.
    fn GetWithOffset(&self, offset: u32) -> u32;
}

#[repr(C)]
struct Value {
    vtable: *const IValueWithOffsetVtbl,
    value: u32,
}

unsafe extern "C" fn get(this: *mut c_void) -> u32 {
    // SAFETY: The test supplies the address of a live `Value`.
    unsafe { (*this.cast::<Value>()).value }
}

unsafe extern "C" fn get_with_offset(this: *mut c_void, offset: u32) -> u32 {
    // SAFETY: The test supplies the address of a live `Value`.
    (unsafe { get(this) }) + offset
}

#[test]
fn inherited_c_table_can_be_called_through_a_borrowed_pointer() {
    let vtable = IValueWithOffsetVtbl {
        base: IValueVtbl { Get: get },
        GetWithOffset: get_with_offset,
    };
    let mut value = Value {
        vtable: &raw const vtable,
        value: 41,
    };
    let pointer = core::ptr::from_mut(&mut value).cast::<c_void>();
    // SAFETY: `pointer` refers to `value`, which stays alive for the entire borrow.
    let interface = unsafe { IValueWithOffset::from_raw_ref(&pointer) };

    assert_eq!(IValueWithOffset::NAME, "IValueWithOffset");
    assert_eq!(raw_of(interface), pointer);
    assert_eq!(vtable_of(interface), &raw const vtable);
    assert_eq!(offset_of!(IValueWithOffsetVtbl, base), 0);
    assert_eq!(size_of::<IValueWithOffset>(), size_of::<*mut c_void>());
    // SAFETY: The object and its vtable are live, and the argument is an ordinary value.
    unsafe {
        assert_eq!(interface.Get(), 41);
        assert_eq!(interface.GetWithOffset(1), 42);
    }
}
