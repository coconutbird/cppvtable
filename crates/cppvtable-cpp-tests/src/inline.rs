//! Calls through embedded C function tables, including inheritance and mixed layouts.

use cppvtable::{OwnedObject, implement, interface};
use std::cell::Cell;
use std::ffi::c_void;

#[interface(abi = c, layout = inline)]
unsafe trait IInlineBase {
    fn get(&self) -> i64;
    fn set(&self, value: i64);
}

#[interface(abi = c, layout = inline, extends(IInlineBase))]
unsafe trait IInlineDerived {
    /// Copy the state into caller-provided storage.
    ///
    /// # Safety
    ///
    /// `output` must point to an aligned, writable `i64` valid through the call.
    unsafe fn write(&self, output: *mut i64);
}

#[interface(abi = c)]
unsafe trait IPointerView {
    fn get(&self) -> i64;
}

#[implement(IInlineDerived, IPointerView)]
struct InlineValue {
    value: Cell<i64>,
}
impl IInlineBaseImpl for InlineValue {
    fn get(&self) -> i64 {
        self.value.get()
    }
    fn set(&self, value: i64) {
        self.value.set(value);
    }
}
impl IInlineDerivedImpl for InlineValue {
    unsafe fn write(&self, output: *mut i64) {
        // SAFETY: The implementation contract requires aligned writable storage.
        unsafe { output.write(self.value.get()) };
    }
}
impl IPointerViewImpl for InlineValue {
    fn get(&self) -> i64 {
        self.value.get()
    }
}

#[implement(IPointerView, IInlineDerived)]
struct PointerFirstValue {
    value: Cell<i64>,
}
impl IInlineBaseImpl for PointerFirstValue {
    fn get(&self) -> i64 {
        self.value.get()
    }
    fn set(&self, value: i64) {
        self.value.set(value);
    }
}
impl IInlineDerivedImpl for PointerFirstValue {
    unsafe fn write(&self, output: *mut i64) {
        // SAFETY: The implementation contract requires aligned writable storage.
        unsafe { output.write(self.value.get()) };
    }
}
impl IPointerViewImpl for PointerFirstValue {
    fn get(&self) -> i64 {
        self.value.get()
    }
}

#[interface(abi = c, layout = inline, slots = 50)]
unsafe trait IInlinePartial {
    #[slot(32)]
    #[abi(convention = "stdcall")]
    fn known(&self, increment: i32) -> i32;
    #[abi(convention = "fastcall")]
    fn update(&self, value: i32);
}

#[implement(IInlinePartial)]
struct PartialValue {
    value: Cell<i32>,
}
impl IInlinePartialImpl for PartialValue {
    fn known(&self, increment: i32) -> i32 {
        self.value.get() + increment
    }
    fn update(&self, value: i32) {
        self.value.set(value);
    }
}

unsafe extern "C" {
    fn cppvtable_c_inline_create(value: i64) -> *mut c_void;
    fn cppvtable_c_inline_delete(object: *mut c_void);
    fn cppvtable_c_inline_get(object: *mut c_void) -> i64;
    fn cppvtable_c_inline_set(object: *mut c_void, value: i64);
    fn cppvtable_c_inline_write(object: *mut c_void, output: *mut i64);
    fn cppvtable_c_inline_native_state(object: *mut c_void) -> i64;
    fn cppvtable_c_inline_pointer_view_get(object: *mut c_void) -> i64;
    fn cppvtable_c_inline_partial_create(value: i32) -> *mut c_void;
    fn cppvtable_c_inline_partial_call(object: *mut c_void, increment: i32) -> i32;
    fn cppvtable_c_inline_partial_update(object: *mut c_void, value: i32);
    fn cppvtable_c_inline_partial_native_state(object: *mut c_void) -> i32;
}

#[test]
fn rust_calls_native_inline_base_and_derived_state_methods() {
    // SAFETY: C allocates the matching derived object.
    let raw = unsafe { cppvtable_c_inline_create(0x1_0000_002a) };
    {
        // SAFETY: The C object lives until the delete below, after the last borrow.
        let interface = unsafe { IInlineDerived::from_raw(raw) }.expect("C allocation succeeded");
        assert_eq!(
            std::ptr::from_ref(interface.vtable()).cast::<c_void>(),
            raw.cast_const()
        );
        assert_eq!(interface.get(), 0x1_0000_002a);
        interface.set(-7);
        // SAFETY: `raw` is the live C object.
        assert_eq!(unsafe { cppvtable_c_inline_native_state(raw) }, -7);
        let mut output = 0;
        // SAFETY: `output` is a writable local.
        unsafe { interface.write(&raw mut output) };
        assert_eq!(output, -7);
        // SAFETY: The base table is the prefix of the same live object.
        let base = unsafe { IInlineBase::from_raw(raw) }.expect("C allocation succeeded");
        assert_eq!(base.get(), -7);
        assert_eq!(
            std::ptr::from_ref(base.vtable()).cast::<c_void>(),
            raw.cast_const()
        );
    }
    // SAFETY: The C allocation is deleted once, after its last borrow.
    unsafe { cppvtable_c_inline_delete(raw) };
}

#[test]
fn native_c_calls_rust_inline_table_and_secondary_pointer_table() {
    let owner = OwnedObject::new(InlineValue {
        value: Cell::new(0x1_0000_002a),
    });
    let inline = owner.as_raw::<IInlineDerived>();
    let pointer = owner.as_raw::<IPointerView>();
    assert_eq!(
        pointer as usize - inline as usize,
        std::mem::size_of::<IInlineDerivedVtbl>()
    );
    let base = owner.try_interface::<IInlineBase>().expect("inline base");
    assert_eq!(base.as_raw(), inline);
    assert_eq!(
        std::ptr::from_ref(base.vtable()).cast::<c_void>(),
        inline.cast_const()
    );
    // SAFETY: The owner keeps both tables alive; local output remains writable.
    unsafe {
        assert_eq!(cppvtable_c_inline_get(inline), 0x1_0000_002a);
        assert_eq!(cppvtable_c_inline_pointer_view_get(pointer), 0x1_0000_002a);
        cppvtable_c_inline_set(inline, -7);
        let mut output = 0;
        cppvtable_c_inline_write(inline, &raw mut output);
        assert_eq!(output, -7);
        assert_eq!(cppvtable_c_inline_pointer_view_get(pointer), -7);
    }
    assert_eq!(owner.value.get(), -7);
}

#[test]
fn native_c_calls_rust_secondary_inline_table_after_primary_pointer_table() {
    let owner = OwnedObject::new(PointerFirstValue {
        value: Cell::new(42),
    });
    let pointer = owner.as_raw::<IPointerView>();
    let inline = owner.as_raw::<IInlineDerived>();
    assert_eq!(
        inline as usize - pointer as usize,
        std::mem::size_of::<usize>()
    );
    let base = owner.try_interface::<IInlineBase>().expect("inline base");
    assert_eq!(base.as_raw(), inline);
    assert_eq!(
        std::ptr::from_ref(base.vtable()).cast::<c_void>(),
        inline.cast_const()
    );
    // SAFETY: The owner keeps both tables alive; local output remains writable.
    unsafe {
        assert_eq!(cppvtable_c_inline_get(inline), 42);
        cppvtable_c_inline_set(inline, 7);
        let mut output = 0;
        cppvtable_c_inline_write(inline, &raw mut output);
        assert_eq!(output, 7);
        assert_eq!(cppvtable_c_inline_pointer_view_get(pointer), 7);
    }
    assert_eq!(owner.value.get(), 7);
}

#[test]
fn rust_calls_native_inline_partial_table_with_mixed_conventions() {
    assert_eq!(
        std::mem::size_of::<IInlinePartialVtbl>(),
        50 * std::mem::size_of::<usize>()
    );
    assert_eq!(
        std::mem::offset_of!(IInlinePartialVtbl, known),
        32 * std::mem::size_of::<usize>()
    );
    // SAFETY: The native table has all 50 entries with matching functions at entries 32 and 33.
    let raw = unsafe { cppvtable_c_inline_partial_create(10) };
    {
        // SAFETY: The C object lives until the delete below, after the last borrow.
        let interface = unsafe { IInlinePartial::from_raw(raw) }.expect("C allocation succeeded");
        assert_eq!(
            std::ptr::from_ref(interface.vtable()).cast::<c_void>(),
            raw.cast_const()
        );
        assert_eq!(interface.known(7), 17);
        interface.update(20);
        assert_eq!(interface.known(7), 27);
        // SAFETY: `raw` is the live C object.
        assert_eq!(unsafe { cppvtable_c_inline_partial_native_state(raw) }, 20);
    }
    // SAFETY: The C allocation is deleted once, after its last borrow.
    unsafe { cppvtable_c_inline_delete(raw) };
}

#[test]
fn native_c_calls_rust_inline_partial_table_with_mixed_conventions() {
    let owner = OwnedObject::new(PartialValue {
        value: Cell::new(10),
    });
    let raw = owner.as_raw::<IInlinePartial>();
    // SAFETY: The owner keeps its 50-entry inline table alive; only declared entries are used.
    unsafe {
        assert_eq!(cppvtable_c_inline_partial_call(raw, 7), 17);
        cppvtable_c_inline_partial_update(raw, 20);
        assert_eq!(cppvtable_c_inline_partial_call(raw, 7), 27);
    }
    assert_eq!(owner.value.get(), 20);
}
