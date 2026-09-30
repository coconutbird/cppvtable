//! Explicit vtable extent counts inherited, known, and reserved entries.

use core::mem::{offset_of, size_of};

use cppvtable::{OwnedObject, implement, interface};

#[interface(abi = c, slots = 50)]
unsafe trait IPartial {
    #[slot(32)]
    fn known(&self) -> u32;
}

#[interface(abi = c, extends(IPartial), slots = 55)]
unsafe trait IDerived {
    #[slot(2)]
    fn extra(&self) -> u32;
}

#[interface(abi = c, extends(IPartial), slots = 51)]
unsafe trait IExact {
    fn extra(&self) -> u32;
}

#[implement(IDerived, IExact)]
struct Partial;

impl IPartialImpl for Partial {
    fn known(&self) -> u32 {
        32
    }
}

impl IDerivedImpl for Partial {
    fn extra(&self) -> u32 {
        52
    }
}

impl IExactImpl for Partial {
    fn extra(&self) -> u32 {
        50
    }
}

#[test]
fn extent_includes_the_entire_base_and_reserves_unknown_entries() {
    let entry = size_of::<unsafe extern "C" fn()>();
    assert_eq!(size_of::<IPartialVtbl>(), 50 * entry);
    assert_eq!(offset_of!(IPartialVtbl, known), 32 * entry);
    assert_eq!(size_of::<IDerivedVtbl>(), 55 * entry);
    assert_eq!(offset_of!(IDerivedVtbl, extra), 52 * entry);
    assert_eq!(size_of::<IExactVtbl>(), 51 * entry);
    assert_eq!(offset_of!(IExactVtbl, extra), 50 * entry);

    let object = OwnedObject::new(Partial);
    let derived = object.interface::<IDerived>();
    let exact = object.interface::<IExact>();
    assert_eq!(derived.known(), 32);
    assert_eq!(derived.extra(), 52);
    assert_eq!(exact.known(), 32);
    assert_eq!(exact.extra(), 50);
    let vtable = derived.vtable();
    assert!(vtable.base.reserved_0.is_none());
    assert!(vtable.base.reserved_31.is_none());
    assert!(vtable.base.__reserved_tail.iter().all(Option::is_none));
    assert!(vtable.reserved_0.is_none());
    assert!(vtable.reserved_1.is_none());
    assert!(vtable.__reserved_tail.iter().all(Option::is_none));
}
