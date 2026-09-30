//! Explicit slots.
//!
//! `#[slot(N)]` puts a method at a fixed index of the derived part of the vtable. The
//! macro fills the space with reserved entries, so the layout is the layout of the
//! foreign header even when the declaration leaves methods out.

use core::ffi::c_void;
use core::mem::{offset_of, size_of};

use cppvtable_com::{ComObject, ComPtr, RefCounted, SingleRefCount, implement, interface};

/// An interface with holes in the slot numbers.
#[interface(abi = com, iid = "51075001-0000-4000-8000-000000000001")]
pub unsafe trait ISparse {
    /// The first method. It takes slot 0 of the derived part.
    fn First(&self) -> u32;
    /// The fourth method. Slots 1 and 2 stay reserved.
    #[slot(3)]
    fn Fourth(&self) -> u32;
    /// The fifth method. It follows without a hole.
    fn Fifth(&self) -> u32;
}

/// A derived interface. Its own slot numbers start again at 0.
#[interface(abi = com, iid = "51075002-0000-4000-8000-000000000002", extends(ISparse))]
pub unsafe trait ISparseChild {
    /// The second method of the derived part. Slot 0 stays reserved.
    #[slot(1)]
    fn Second(&self) -> u32;
}

/// The object.
#[implement(ISparseChild)]
struct Sparse;

// SAFETY: Hooks obey the reference-count contract and all returned pointers stay live.
unsafe impl RefCounted for Sparse {
    type Policy = SingleRefCount;
}

impl ISparseImpl for Sparse {
    fn First(&self) -> u32 {
        1
    }

    fn Fourth(&self) -> u32 {
        4
    }

    fn Fifth(&self) -> u32 {
        5
    }
}

impl ISparseChildImpl for Sparse {
    fn Second(&self) -> u32 {
        20
    }
}

#[test]
fn a_reserved_entry_keeps_the_slot_number_of_the_header() {
    let pointer = size_of::<usize>();
    // 3 slots of `IUnknown` and 5 slots of `ISparse`.
    assert_eq!(size_of::<ISparseVtbl>(), 8 * pointer);
    assert_eq!(offset_of!(ISparseVtbl, First), 3 * pointer);
    assert_eq!(offset_of!(ISparseVtbl, reserved_1), 4 * pointer);
    assert_eq!(offset_of!(ISparseVtbl, reserved_2), 5 * pointer);
    assert_eq!(offset_of!(ISparseVtbl, Fourth), 6 * pointer);
    assert_eq!(offset_of!(ISparseVtbl, Fifth), 7 * pointer);

    // 8 slots of `ISparse` and 2 slots of `ISparseChild`.
    assert_eq!(size_of::<ISparseChildVtbl>(), 10 * pointer);
    assert_eq!(offset_of!(ISparseChildVtbl, reserved_0), 8 * pointer);
    assert_eq!(offset_of!(ISparseChildVtbl, Second), 9 * pointer);
}

#[test]
fn the_static_vtable_holds_a_null_pointer_in_a_reserved_entry() {
    let object: ComPtr<ISparseChild> = ComObject::new(Sparse);
    let this = object.as_raw();
    // SAFETY: `this` is a valid interface pointer of `ISparseChild`.
    let vtable = unsafe { &*(*this.cast::<*const ISparseChildVtbl>()) };
    assert!(vtable.base.reserved_1.is_none());
    assert!(vtable.base.reserved_2.is_none());
    assert!(vtable.reserved_0.is_none());
}

#[test]
fn a_c_caller_finds_each_method_at_its_own_slot_number() {
    let object: ComPtr<ISparseChild> = ComObject::new(Sparse);
    let this = object.as_raw();
    // SAFETY: `this` is a valid COM interface pointer, so the first field is the vtable.
    let slots = unsafe { *this.cast::<*const *const c_void>() };

    // Read the raw slot and call it with the signature of the method.
    for (index, expected) in [(3_usize, 1_u32), (6, 4), (7, 5), (9, 20)] {
        // SAFETY: The vtable of the object has 10 slots.
        let raw = unsafe { *slots.add(index) };
        // SAFETY: Each of these slots holds a method with the signature
        // `fn(*mut c_void) -> u32`.
        let method: unsafe extern "system" fn(*mut c_void) -> u32 =
            unsafe { core::mem::transmute(raw) };
        // SAFETY: The object is alive and the pointer is its interface pointer.
        assert_eq!(unsafe { method(this) }, expected);
    }

    // The reserved slots hold a null pointer.
    for index in [4_usize, 5, 8] {
        // SAFETY: The vtable of the object has 10 slots.
        let raw = unsafe { *slots.add(index) };
        assert!(raw.is_null());
    }
}

#[test]
fn the_safe_wrapper_reaches_the_same_methods() {
    let object: ComPtr<ISparseChild> = ComObject::new(Sparse);
    // SAFETY: The object is alive.
    unsafe {
        assert_eq!(object.Second(), 20);
        assert_eq!(object.First(), 1);
        assert_eq!(object.Fourth(), 4);
        assert_eq!(object.Fifth(), 5);
    }
}
