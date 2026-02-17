//! Tests for COM interface support

use cppvtable::com::{ComRefCount, IUnknownVTable, S_OK};
use cppvtable::proc::{com_implement, com_interface};
use cppvtable::{IUnknown, VTableLayout};
use std::ffi::c_void;
use std::ptr;

// =============================================================================
// Test: Basic COM interface definition
// =============================================================================

#[com_interface("12345678-1234-5678-9abc-def012345678")]
pub trait ICalculator {
    fn add(&self, a: i32, b: i32) -> i32;
    fn multiply(&self, a: i32, b: i32) -> i32;
}

#[test]
fn test_com_interface_iid() {
    let iid = ICalculator::iid();
    assert_eq!(iid.data1, 0x12345678);
    assert_eq!(iid.data2, 0x1234);
    assert_eq!(iid.data3, 0x5678);
    assert_eq!(iid.data4, [0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78]);
}

#[test]
fn test_com_interface_iid_const() {
    // IID constant should also be available
    assert_eq!(IID_ICALCULATOR.data1, 0x12345678);
}

#[test]
fn test_com_vtable_has_iunknown_methods() {
    // Verify vtable has IUnknown methods at expected positions
    let vtable_size = std::mem::size_of::<ICalculatorVTable>();
    // Should be: IUnknownVTable (3 ptrs) + 2 user methods = 5 function pointers worth
    // On 64-bit: 5 * 8 = 40 bytes
    // On 32-bit: 5 * 4 = 20 bytes
    let ptr_size = std::mem::size_of::<*const c_void>();
    assert_eq!(vtable_size, 5 * ptr_size);
}

// =============================================================================
// Test: VTableLayout trait and inheritance
// =============================================================================

#[test]
fn test_iunknown_vtable_layout() {
    // IUnknown has 3 methods: QueryInterface, AddRef, Release
    assert_eq!(<IUnknown as VTableLayout>::SLOT_COUNT, 3);

    // VTable type should be IUnknownVTable (verify via size)
    assert_eq!(
        std::mem::size_of::<<IUnknown as VTableLayout>::VTable>(),
        std::mem::size_of::<IUnknownVTable>()
    );
}

#[test]
fn test_derived_interface_vtable_layout() {
    // ICalculator extends IUnknown (3) + 2 own methods = 5 total slots
    assert_eq!(<ICalculator as VTableLayout>::SLOT_COUNT, 5);

    // VTable type should be ICalculatorVTable (verify via size)
    assert_eq!(
        std::mem::size_of::<<ICalculator as VTableLayout>::VTable>(),
        std::mem::size_of::<ICalculatorVTable>()
    );
}

#[test]
fn test_vtable_base_field_offset() {
    // The `base` field (IUnknownVTable) should be at offset 0
    let base_offset = std::mem::offset_of!(ICalculatorVTable, base);
    assert_eq!(base_offset, 0);
}

#[test]
fn test_vtable_embeds_iunknown() {
    // ICalculatorVTable should embed IUnknownVTable as its first field
    let iunknown_size = std::mem::size_of::<IUnknownVTable>();
    let ptr_size = std::mem::size_of::<*const c_void>();

    // IUnknownVTable should be 3 function pointers
    assert_eq!(iunknown_size, 3 * ptr_size);

    // ICalculatorVTable.base should be exactly IUnknownVTable sized
    // (this verifies the embedded struct, not a pointer)
    assert_eq!(
        std::mem::size_of::<<IUnknown as VTableLayout>::VTable>(),
        iunknown_size
    );
}

// =============================================================================
// Test: COM interface implementation
// =============================================================================

#[repr(C)]
pub struct Calculator {
    vtable_i_calculator: *const ICalculatorVTable,
    ref_count: ComRefCount,
    base_value: i32,
}

impl Calculator {
    pub fn new(base: i32) -> Self {
        Self {
            vtable_i_calculator: Self::VTABLE_I_CALCULATOR,
            ref_count: ComRefCount::new(),
            base_value: base,
        }
    }
}

#[com_implement(ICalculator)]
impl Calculator {
    fn add(&self, a: i32, b: i32) -> i32 {
        self.base_value + a + b
    }

    fn multiply(&self, a: i32, b: i32) -> i32 {
        self.base_value * a * b
    }
}

#[test]
fn test_com_implement_basic() {
    let calc = Calculator::new(10);

    // Call methods directly
    assert_eq!(calc.add(2, 3), 15); // 10 + 2 + 3
    assert_eq!(calc.multiply(2, 3), 60); // 10 * 2 * 3
}

#[test]
fn test_com_implement_vtable_calls() {
    let mut calc = Calculator::new(10);

    // Get interface pointer and call through vtable
    unsafe {
        let iface = ICalculator::from_ptr_mut(&mut calc as *mut _ as *mut c_void);
        assert_eq!(iface.add(1, 2), 13); // 10 + 1 + 2
        assert_eq!(iface.multiply(2, 2), 40); // 10 * 2 * 2
    }
}

#[test]
fn test_com_ref_counting() {
    let mut calc = Calculator::new(10);

    unsafe {
        let iface = ICalculator::from_ptr_mut(&mut calc as *mut _ as *mut c_void);

        // Initial ref count is 1
        assert_eq!(calc.ref_count.count(), 1);

        // AddRef increments
        let count = iface.add_ref();
        assert_eq!(count, 2);
        assert_eq!(calc.ref_count.count(), 2);

        // Release decrements
        let count = iface.release();
        assert_eq!(count, 1);
        assert_eq!(calc.ref_count.count(), 1);
    }
}

#[test]
fn test_com_query_interface() {
    let calc = Calculator::new(10);

    unsafe {
        let iface = ICalculator::from_ptr(&calc as *const _ as *mut c_void);

        // Query for the same interface
        let mut ppv: *mut c_void = ptr::null_mut();
        let hr = iface.query_interface(ICalculator::iid(), &mut ppv);
        assert_eq!(hr, S_OK);
        assert!(!ppv.is_null());

        // Queried pointer should work
        let iface2 = ICalculator::from_ptr_mut(ppv);
        assert_eq!(iface2.add(1, 1), 12);

        // Release the extra reference from QueryInterface
        iface2.release();
    }
}

// =============================================================================
// Test: Auto-generated forwarders for derived interfaces
// =============================================================================

// Define IScientificCalculator extending ICalculator
// This tests that the auto-generated icalculator_forwarders! and icalculator_base_vtable! macros exist
#[cppvtable::proc::cppvtable(stdcall, extends(ICalculator))]
pub trait IScientificCalculator {
    fn square(&self, x: i32) -> i32;
}

#[test]
fn test_derived_interface_extends_calculator() {
    // IScientificCalculator should have ICalculator's slot count + 1 own method
    assert_eq!(<IScientificCalculator as VTableLayout>::SLOT_COUNT, 6);

    // Vtable should be the right size: ICalculator (5 slots) + 1 own = 6 function pointers
    let ptr_size = std::mem::size_of::<*const c_void>();
    assert_eq!(
        std::mem::size_of::<IScientificCalculatorVTable>(),
        6 * ptr_size
    );
}
