//! Inline C callback headers, partial inheritance, and mixed interface storage.

use core::ffi::c_void;
use core::mem::{align_of, offset_of, size_of};
use std::cell::Cell;
use std::rc::Rc;

use cppvtable::{Object, OwnedObject, implement, interface, interface_of};
use cppvtable_abi::interface::{raw_of, vtable_of};

/// The address a view's vtable reference points at.
fn table_address<T>(table: &T) -> *mut c_void {
    core::ptr::from_ref(table).cast::<c_void>().cast_mut()
}

#[interface(abi = c, layout = inline, slots = 3)]
unsafe trait IInlineBase {
    #[slot(1)]
    fn value(&self) -> u32;
}

#[interface(abi = c, layout = inline, extends(IInlineBase), slots = 6)]
unsafe trait IInlineDerived {
    #[slot(1)]
    fn add(&self, amount: u32) -> u32;
}

// Omitting `layout` preserves the ordinary pointer-to-vtable contract.
#[interface(abi = c)]
unsafe trait IPointer {
    fn set(&self, value: u32);
    /// Return this allocation's inline interface pointer.
    ///
    /// # Safety
    ///
    /// `self` must be the implementation field of its live `Object` allocation.
    unsafe fn inline_identity(&self) -> *mut c_void;
}

#[implement(IInlineDerived, IPointer)]
#[repr(align(64))]
struct InlineFirst {
    value: Cell<u32>,
    drops: Rc<Cell<u32>>,
}

impl IInlineBaseImpl for InlineFirst {
    fn value(&self) -> u32 {
        self.value.get()
    }
}

impl IInlineDerivedImpl for InlineFirst {
    fn add(&self, amount: u32) -> u32 {
        self.value.get() + amount
    }
}

impl IPointerImpl for InlineFirst {
    fn set(&self, value: u32) {
        self.value.set(value);
    }

    unsafe fn inline_identity(&self) -> *mut c_void {
        // SAFETY: The method contract requires an implementation embedded in `Object`.
        unsafe { interface_of::<IInlineDerived>(self) }.as_raw()
    }
}

impl Drop for InlineFirst {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[implement(IPointer, IInlineDerived)]
struct PointerFirst {
    value: Cell<u32>,
}

impl IInlineBaseImpl for PointerFirst {
    fn value(&self) -> u32 {
        self.value.get()
    }
}

impl IInlineDerivedImpl for PointerFirst {
    fn add(&self, amount: u32) -> u32 {
        self.value.get() + amount
    }
}

impl IPointerImpl for PointerFirst {
    fn set(&self, value: u32) {
        self.value.set(value);
    }

    unsafe fn inline_identity(&self) -> *mut c_void {
        // SAFETY: The method contract requires an implementation embedded in `Object`.
        unsafe { interface_of::<IInlineDerived>(self) }.as_raw()
    }
}

#[test]
fn inherited_partial_inline_table_is_the_interface_header() {
    let entry = size_of::<unsafe extern "C" fn()>();
    assert_eq!(size_of::<IInlineBaseVtbl>(), 3 * entry);
    assert_eq!(offset_of!(IInlineBaseVtbl, value), entry);
    assert_eq!(size_of::<IInlineDerivedVtbl>(), 6 * entry);
    assert_eq!(offset_of!(IInlineDerivedVtbl, add), 4 * entry);

    let owner = OwnedObject::new(InlineFirst {
        value: Cell::new(17),
        drops: Rc::new(Cell::new(0)),
    });
    let inline = owner.interface::<IInlineDerived>();
    let base = owner.try_interface::<IInlineBase>().unwrap();
    assert_eq!(table_address(inline.vtable()), inline.as_raw());
    assert!(core::ptr::eq(vtable_of(&*inline), inline.vtable()));
    assert_eq!(raw_of(&*inline), inline.as_raw());
    assert_eq!(base.as_raw(), inline.as_raw());
    assert_eq!(table_address(base.vtable()), base.as_raw());

    assert_eq!(inline.value(), 17);
    assert_eq!(inline.add(3), 20);
    assert_eq!(base.value(), 17);
    let header = inline.vtable();
    assert!(header.base.reserved_0.is_none());
    assert!(header.base.__reserved_tail.iter().all(Option::is_none));
    assert!(header.reserved_0.is_none());
    assert!(header.__reserved_tail.iter().all(Option::is_none));
}

#[test]
fn pointer_and_inline_chains_adjust_to_the_same_value_in_either_order() {
    let entry = size_of::<unsafe extern "C" fn()>();
    let inline_first = OwnedObject::new(InlineFirst {
        value: Cell::new(17),
        drops: Rc::new(Cell::new(0)),
    });
    let pointer_first = OwnedObject::new(PointerFirst {
        value: Cell::new(17),
    });

    assert_eq!(Object::<InlineFirst>::slot_offset(1), 6 * entry);
    assert_eq!(Object::<PointerFirst>::slot_offset(1), size_of::<usize>());
    let inline = inline_first.interface::<IInlineDerived>();
    let pointer = inline_first.interface::<IPointer>();
    let other_inline = pointer_first.interface::<IInlineDerived>();
    let other_pointer = pointer_first.interface::<IPointer>();
    assert_eq!(
        pointer.as_raw() as usize - inline.as_raw() as usize,
        6 * entry
    );
    assert_eq!(
        other_inline.as_raw() as usize - other_pointer.as_raw() as usize,
        size_of::<usize>()
    );
    assert_ne!(table_address(pointer.vtable()), pointer.as_raw());
    assert_eq!(table_address(other_inline.vtable()), other_inline.as_raw());

    // SAFETY: Both owners remain alive, and each shim supplies its implementation
    // from the allocation, as `inline_identity` requires.
    unsafe {
        assert_eq!(pointer.inline_identity(), inline.as_raw());
        assert_eq!(other_pointer.inline_identity(), other_inline.as_raw());
    }
    pointer.set(40);
    other_pointer.set(80);
    assert_eq!(inline.value(), 40);
    assert_eq!(inline.add(2), 42);
    assert_eq!(other_inline.value(), 80);
    assert_eq!(other_inline.add(2), 82);
}

#[test]
fn aligned_inline_data_survives_ownership_transfer_and_drops_once() {
    let drops = Rc::new(Cell::new(0));
    let owner = OwnedObject::new(InlineFirst {
        value: Cell::new(41),
        drops: Rc::clone(&drops),
    });
    assert_eq!(align_of::<InlineFirst>(), 64);
    assert_eq!(core::ptr::from_ref::<InlineFirst>(&owner) as usize % 64, 0);
    let inline = owner.as_raw::<IInlineDerived>();
    let pointer = owner.as_raw::<IPointer>();
    let moved = Box::new(owner);
    assert_eq!(moved.as_raw::<IInlineDerived>(), inline);
    assert_eq!(moved.as_raw::<IPointer>(), pointer);
    let raw = (*moved).into_raw();
    assert_eq!(raw.cast::<c_void>(), inline);
    assert_eq!(drops.get(), 0);
    // SAFETY: `raw` uniquely owns the allocation relinquished above.
    let restored = unsafe { OwnedObject::from_raw(raw) };
    assert_eq!(
        core::ptr::from_ref::<InlineFirst>(&restored) as usize % 64,
        0
    );
    assert_eq!(restored.interface::<IInlineDerived>().add(1), 42);
    drop(restored);
    assert_eq!(drops.get(), 1);
}

#[interface(abi = c, layout = inline)]
unsafe trait IForeignInline {
    fn read(&self) -> u32;
}

#[repr(C)]
struct ForeignInline {
    header: IForeignInlineVtbl,
    value: u32,
}

unsafe extern "C" fn foreign_read(this: *mut c_void) -> u32 {
    // SAFETY: The foreign fixture supplies the address of its live containing value.
    unsafe { (*this.cast::<ForeignInline>()).value }
}

#[test]
fn borrowed_foreign_inline_table_calls_its_embedded_function_entry() {
    let mut foreign = ForeignInline {
        header: IForeignInlineVtbl { read: foreign_read },
        value: 42,
    };
    let raw = core::ptr::from_mut(&mut foreign).cast::<c_void>();
    // SAFETY: `raw` identifies this immutable header, which remains alive throughout
    // the borrow, together with the data accessed by its callback.
    let view = unsafe { IForeignInline::from_raw(raw) }.unwrap();
    assert_eq!(table_address(view.vtable()), raw);
    assert_eq!(view.read(), 42);
}
