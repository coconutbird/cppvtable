//! Tests for #[slot(N)] attribute - explicit vtable slot indices

use cppvtable::proc::{cpp_interface, implement};
use std::ffi::c_void;

/// Interface with explicit slot indices
/// Layout: slot 0, slot 1, slots 2-4 reserved, slot 5, slot 6
#[cpp_interface]
pub trait ISlotted {
    fn at_slot_0(&self) -> i32;
    fn at_slot_1(&self) -> i32;
    #[slot(5)]
    fn at_slot_5(&self) -> i32;
    fn at_slot_6(&self) -> i32;
}

#[test]
fn test_vtable_size_with_gaps() {
    // 7 slots (0, 1, 2, 3, 4, 5, 6) = 7 function pointers
    let size = std::mem::size_of::<ISlottedVTable>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(size, 56, "7 slots = 56 bytes on x64");
    #[cfg(target_pointer_width = "32")]
    assert_eq!(size, 28, "7 slots = 28 bytes on x86");
}

#[repr(C)]
pub struct SlotTester {
    vtable_i_slotted: *const ISlottedVTable,
    id: i32,
}

#[implement(ISlotted)]
impl SlotTester {
    fn at_slot_0(&self) -> i32 {
        0
    }
    fn at_slot_1(&self) -> i32 {
        1
    }
    #[slot(5)]
    fn at_slot_5(&self) -> i32 {
        5
    }
    fn at_slot_6(&self) -> i32 {
        6
    }
}

impl SlotTester {
    pub fn new() -> Self {
        SlotTester {
            vtable_i_slotted: Self::VTABLE_I_SLOTTED,
            id: 42,
        }
    }
}

#[test]
fn test_slot_methods_return_correct_values() {
    let tester = SlotTester::new();

    assert_eq!(tester.at_slot_0(), 0);
    assert_eq!(tester.at_slot_1(), 1);
    assert_eq!(tester.at_slot_5(), 5);
    assert_eq!(tester.at_slot_6(), 6);
}

#[test]
fn test_vtable_slot_order() {
    let tester = SlotTester::new();

    unsafe {
        let vtable = &*tester.vtable_i_slotted;
        let this = &tester as *const SlotTester as *mut c_void;

        // Call each slot and verify return value matches slot index
        assert_eq!((vtable.at_slot_0)(this), 0);
        assert_eq!((vtable.at_slot_1)(this), 1);
        // Slots 2-4 are reserved (would panic if called)
        assert_eq!((vtable.at_slot_5)(this), 5);
        assert_eq!((vtable.at_slot_6)(this), 6);
    }
}

/// Test interface starting with non-zero slot
#[cpp_interface]
pub trait IStartsAtThree {
    #[slot(3)]
    fn first_method(&self) -> i32;
    fn second_method(&self) -> i32;
}

#[test]
fn test_interface_starting_at_slot_3() {
    // Slots 0, 1, 2, 3, 4 = 5 function pointers
    let size = std::mem::size_of::<IStartsAtThreeVTable>();
    #[cfg(target_pointer_width = "64")]
    assert_eq!(size, 40, "5 slots = 40 bytes on x64");
}

#[repr(C)]
pub struct StartsAtThreeTester {
    vtable_i_starts_at_three: *const IStartsAtThreeVTable,
}

#[implement(IStartsAtThree)]
impl StartsAtThreeTester {
    #[slot(3)]
    fn first_method(&self) -> i32 {
        3
    }
    fn second_method(&self) -> i32 {
        4
    }
}

impl StartsAtThreeTester {
    pub fn new() -> Self {
        StartsAtThreeTester {
            vtable_i_starts_at_three: Self::VTABLE_I_STARTS_AT_THREE,
        }
    }
}

#[test]
fn test_starts_at_three() {
    let tester = StartsAtThreeTester::new();

    assert_eq!(tester.first_method(), 3);
    assert_eq!(tester.second_method(), 4);
}
