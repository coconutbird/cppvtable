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
    fn identity(&self) -> *mut c_void;
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
    fn identity(&self) -> *mut c_void {
        unsafe { interface_of::<Self, IDerived>(self) }
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
    assert_eq!(unsafe { primary.value() }, 17);
    assert_eq!(unsafe { primary.add(3) }, 20);
    assert_eq!(unsafe { secondary.identity() }, primary.as_raw());
    unsafe {
        secondary.set(40);
    }
    assert_eq!(unsafe { primary.value() }, 40);
    let base = owner.query_interface::<IBase>().unwrap();
    assert_eq!(base.as_raw(), primary.as_raw());
    assert_eq!(unsafe { base.value() }, 40);
    assert!(owner.query_interface::<IUnsupported>().is_none());
    let vtable = unsafe { &*secondary.vtable() };
    assert!(vtable.reserved_1.is_none());
    assert!(vtable.reserved_2.is_none());
    drop(owner);
    assert_eq!(drops.get(), 1);
}

#[test]
fn moving_owner_and_transferring_raw_ownership_preserve_address_and_drop_once() {
    let drops = Rc::new(Cell::new(0));
    let owner = Object::new(Counter {
        value: Cell::new(5),
        drops: drops.clone(),
    });
    let primary = owner.as_raw::<IDerived>();
    let moved = Box::new(owner);
    assert_eq!(primary, moved.as_raw::<IDerived>());
    let raw = (*moved).into_raw();
    assert_eq!(raw.cast::<c_void>(), primary);
    assert_eq!(drops.get(), 0);
    let restored = unsafe { OwnedObject::from_raw(raw) };
    assert_eq!(unsafe { restored.interface::<IDerived>().value() }, 5);
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
