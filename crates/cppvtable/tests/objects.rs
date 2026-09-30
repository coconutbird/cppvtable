//! Ordinary object lifetime, interface inheritance, and multiple chain adjustment.

use core::ffi::c_void;
use core::mem::size_of;
use std::cell::Cell;
use std::rc::Rc;

use cppvtable::{Object, OwnedObject, implement, interface, interface_of};

#[interface(abi = c, root)]
unsafe trait IBase {
    fn value(&self) -> u32;
}

#[interface(abi = c, extends(IBase))]
unsafe trait IDerived {
    fn add(&self, value: u32) -> u32;
}

#[interface(abi = c)]
unsafe trait ISecondary {
    /// Return this object's primary interface pointer.
    ///
    /// # Safety
    ///
    /// `self` must be the implementation field of a live `Object<Counter>`.
    unsafe fn identity(&self) -> *mut c_void;
    #[slot(3)]
    fn set(&self, value: u32);
}

#[interface(abi = c)]
unsafe trait IUnsupported {
    fn absent(&self);
}

#[implement(IDerived, ISecondary)]
struct Counter {
    value: Cell<u32>,
    drops: Rc<Cell<u32>>,
}

impl IBaseImpl for Counter {
    fn value(&self) -> u32 {
        self.value.get()
    }
}

impl IDerivedImpl for Counter {
    fn add(&self, value: u32) -> u32 {
        self.value.get() + value
    }
}

impl ISecondaryImpl for Counter {
    unsafe fn identity(&self) -> *mut c_void {
        // SAFETY: The caller guarantees `self` is embedded in a live object.
        unsafe { interface_of::<IDerived>(self) }.as_raw()
    }
    fn set(&self, value: u32) {
        self.value.set(value);
    }
}

impl Drop for Counter {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn inherited_and_secondary_vtables_recover_the_same_implementation() {
    let drops = Rc::new(Cell::new(0));
    let owner = OwnedObject::new(Counter {
        value: Cell::new(17),
        drops: drops.clone(),
    });
    let primary = owner.interface::<IDerived>();
    let secondary = owner.interface::<ISecondary>();
    assert_eq!(
        secondary.as_raw() as usize - primary.as_raw() as usize,
        size_of::<usize>()
    );
    assert_eq!(primary.value(), 17);
    assert_eq!(primary.add(3), 20);
    // SAFETY: `secondary` belongs to the live `Object<Counter>`.
    assert_eq!(unsafe { secondary.identity() }, primary.as_raw());
    // SAFETY: The value is borrowed from its live allocation, as identity requires.
    assert_eq!(
        unsafe { ISecondaryImpl::identity(&*owner) },
        primary.as_raw()
    );
    secondary.set(40);
    assert_eq!(primary.value(), 40);
    let base = owner.try_interface::<IBase>().unwrap();
    assert_eq!(base.as_raw(), primary.as_raw());
    assert_eq!(base.value(), 40);
    assert!(owner.try_interface::<IUnsupported>().is_none());
    let vtable = secondary.vtable();
    assert!(vtable.reserved_1.is_none());
    assert!(vtable.reserved_2.is_none());
    drop(owner);
    assert_eq!(drops.get(), 1);
}

#[test]
fn moving_owner_and_transferring_raw_ownership_preserve_address_and_drop_once() {
    let drops = Rc::new(Cell::new(0));
    let owner = OwnedObject::new(Counter {
        value: Cell::new(5),
        drops: drops.clone(),
    });
    let primary = owner.as_raw::<IDerived>();
    let moved = Box::new(owner);
    assert_eq!(primary, moved.as_raw::<IDerived>());
    let raw = (*moved).into_raw();
    assert_eq!(raw.cast::<c_void>(), primary);
    assert_eq!(drops.get(), 0);
    // SAFETY: `raw` came from `into_raw` and is restored once.
    let restored = unsafe { OwnedObject::from_raw(raw) };
    assert_eq!(restored.interface::<IDerived>().value(), 5);
    drop(restored);
    assert_eq!(drops.get(), 1);
}

#[test]
fn plain_object_layout_has_no_com_state() {
    assert_eq!(Object::<Counter>::slot_offset(0), 0);
    assert_eq!(Object::<Counter>::slot_offset(1), size_of::<usize>());
    assert_eq!(
        size_of::<Object<Counter>>(),
        2 * size_of::<usize>() + size_of::<Counter>()
    );
}
