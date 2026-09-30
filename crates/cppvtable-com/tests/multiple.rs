//! More than one interface chain on one object.
//!
//! The object holds one vtable pointer for each chain. A shim of a chain moves the
//! `this` pointer back to the start of the allocation before it calls the method. This
//! is the `this` adjustment of a C++ object with more than one base class.

use core::ffi::c_void;
use core::mem::size_of;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, Ordering};

use cppvtable_com::{
    ComObject, ComPtr, IUnknown, RefCounted, SingleRefCount, implement, interface, interface_of,
};
use cppvtable_com::{GUID, Interface};

/// The base of the first chain.
#[interface(abi = com, iid = "a1a1a1a1-0000-4000-8000-000000000001")]
pub unsafe trait IAlpha {
    /// Give the alpha value.
    fn AlphaValue(&self) -> u32;
}

/// The leaf of the first chain.
#[interface(abi = com, iid = "a1a1a1a1-0000-4000-8000-000000000002", extends(IAlpha))]
pub unsafe trait IAlphaChild {
    /// Give the value of the child.
    fn ChildValue(&self) -> u32;
}

/// The second chain.
#[interface(abi = com, iid = "b2b2b2b2-0000-4000-8000-000000000001")]
pub unsafe trait IBeta {
    /// Give the beta value.
    fn BetaValue(&self) -> u32;
}

/// The third chain.
#[interface(abi = com, iid = "c3c3c3c3-0000-4000-8000-000000000001")]
pub unsafe trait IGamma {
    /// Give the gamma value.
    fn GammaValue(&self) -> u32;
}

/// An identifier that the object answers with the hook `query_extra`.
const EXTRA_IID: GUID = GUID::from_values(0xeeee_0001, 0, 0x4000, [0x80, 0, 0, 0, 0, 0, 0, 1]);

/// An object with three interface chains.
#[implement(IAlphaChild, IBeta, IGamma)]
struct Multi {
    /// The value that each method reads. The `this` adjustment must find it.
    value: AtomicU32,
}

// SAFETY: Hooks obey the reference-count contract and all returned pointers stay live.
unsafe impl RefCounted for Multi {
    type Policy = SingleRefCount;

    unsafe fn query_extra(&self, iid: &GUID) -> Option<NonNull<c_void>> {
        if *iid != EXTRA_IID {
            return None;
        }
        // The hook must add the reference of the pointer that it gives.
        // SAFETY: `self` is the value inside a live object, because the caller is the
        // generated `QueryInterface` of that object.
        let pointer = unsafe { ComPtr::<IBeta>::from_impl(self) };
        NonNull::new(pointer.into_raw())
    }
}

impl IAlphaImpl for Multi {
    fn AlphaValue(&self) -> u32 {
        self.value.load(Ordering::Relaxed) + 10
    }
}

impl IAlphaChildImpl for Multi {
    fn ChildValue(&self) -> u32 {
        self.value.load(Ordering::Relaxed) + 20
    }
}

impl IBetaImpl for Multi {
    fn BetaValue(&self) -> u32 {
        self.value.load(Ordering::Relaxed) + 30
    }
}

impl IGammaImpl for Multi {
    fn GammaValue(&self) -> u32 {
        self.value.load(Ordering::Relaxed) + 40
    }
}

/// Make a new object with the value 1.
fn new_multi() -> ComPtr<IAlphaChild> {
    ComObject::new(Multi {
        value: AtomicU32::new(1),
    })
}

#[test]
fn each_chain_has_its_own_vtable_pointer_in_the_object() {
    let object = new_multi();
    let value = object.as_impl::<Multi>().unwrap();
    let pointer = size_of::<*const c_void>();

    // SAFETY: `value` is the Rust value inside a live object.
    let alpha = unsafe { interface_of::<Multi, IAlphaChild>(value) };
    // SAFETY: See above.
    let beta = unsafe { interface_of::<Multi, IBeta>(value) };
    // SAFETY: See above.
    let gamma = unsafe { interface_of::<Multi, IGamma>(value) };

    assert_eq!(alpha, object.as_raw());
    assert_eq!(beta as usize - alpha as usize, pointer);
    assert_eq!(gamma as usize - alpha as usize, 2 * pointer);
}

#[test]
fn the_this_adjustment_of_each_chain_finds_the_same_value() {
    let object = new_multi();
    let beta: ComPtr<IBeta> = object.cast().unwrap();
    let gamma: ComPtr<IGamma> = object.cast().unwrap();

    assert_eq!(object.ChildValue(), 21);
    assert_eq!(object.AlphaValue(), 11);
    assert_eq!(beta.BetaValue(), 31);
    assert_eq!(gamma.GammaValue(), 41);

    // A change through one chain is visible through every other chain.
    object.as_impl::<Multi>().unwrap();
    beta.as_impl::<Multi>()
        .unwrap()
        .value
        .store(100, Ordering::Relaxed);
    assert_eq!(object.ChildValue(), 120);
    assert_eq!(gamma.GammaValue(), 140);
}

#[test]
fn query_interface_answers_every_chain_and_every_ancestor() {
    let object = new_multi();
    let this = object.as_raw();

    // The first chain and its ancestors give the pointer of the first chain.
    for pointer in [
        object.cast::<IAlphaChild>().unwrap().as_raw(),
        object.cast::<IAlpha>().unwrap().as_raw(),
        object.cast::<IUnknown>().unwrap().as_raw(),
    ] {
        assert_eq!(pointer, this);
    }

    // The other chains give their own pointers.
    let beta = object.cast::<IBeta>().unwrap();
    let gamma = object.cast::<IGamma>().unwrap();
    assert_ne!(beta.as_raw(), this);
    assert_ne!(gamma.as_raw(), this);
    assert_ne!(beta.as_raw(), gamma.as_raw());

    // The identity rule holds from every chain.
    assert_eq!(beta.cast::<IUnknown>().unwrap().as_raw(), this);
    assert_eq!(gamma.cast::<IUnknown>().unwrap().as_raw(), this);
}

#[test]
fn as_impl_works_from_every_chain_and_refuses_another_type() {
    let object = new_multi();
    let beta = object.cast::<IBeta>().unwrap();
    let gamma = object.cast::<IGamma>().unwrap();

    let from_alpha = object.as_impl::<Multi>().unwrap();
    let from_beta = beta.as_impl::<Multi>().unwrap();
    let from_gamma = gamma.as_impl::<Multi>().unwrap();
    assert!(core::ptr::eq(from_alpha, from_beta));
    assert!(core::ptr::eq(from_alpha, from_gamma));
    assert!(object.as_impl::<Other>().is_none());
}

/// Another implementation type. `as_impl` must not answer for it.
#[implement(IGamma, refcount = single)]
struct Other;

impl IGammaImpl for Other {
    fn GammaValue(&self) -> u32 {
        999
    }
}

#[test]
fn the_hook_query_extra_answers_an_identifier_that_the_table_does_not_hold() {
    let object = new_multi();
    let count_before = object.public_count_of::<Multi>().unwrap();

    let iid = EXTRA_IID;
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: The object is alive and the two pointers refer to local values.
    let result = unsafe { object.QueryInterface(&raw const iid, &raw mut out) };
    assert!(result.is_ok());
    assert!(!out.is_null());

    // SAFETY: The hook added the reference that this `ComPtr` owns.
    let extra = unsafe { ComPtr::<IBeta>::from_raw(out) }.unwrap();
    assert_eq!(extra.BetaValue(), 31);
    assert_eq!(
        object.public_count_of::<Multi>(),
        Some(count_before + 1),
        "the hook must add the reference of the pointer that it gives"
    );
}

/// The first of two types with exactly the same shape and the same method bodies.
#[implement(IGamma, refcount = single)]
struct TwinA {
    /// The value of the twin.
    value: AtomicU32,
}

impl IGammaImpl for TwinA {
    fn GammaValue(&self) -> u32 {
        self.value.load(Ordering::Relaxed)
    }
}

/// The second of the two twins. The machine code of the two is the same.
#[implement(IGamma, refcount = single)]
struct TwinB {
    /// The value of the twin.
    value: AtomicU32,
}

impl IGammaImpl for TwinB {
    fn GammaValue(&self) -> u32 {
        self.value.load(Ordering::Relaxed)
    }
}

#[test]
fn two_types_with_the_same_shape_keep_two_vtables() {
    // `as_impl` compares the vtable pointer of an object with the address of the static
    // vtable of the type. The test is only correct while two types never share one
    // static. This test runs with optimization on as well, so it also checks that the
    // compiler and the linker do not fold the two statics into one.
    let first: ComPtr<IGamma> = ComObject::new(TwinA {
        value: AtomicU32::new(7),
    });
    let second: ComPtr<IGamma> = ComObject::new(TwinB {
        value: AtomicU32::new(8),
    });

    let first_vtable = <TwinA as cppvtable_com::ComImplement>::vtable_slots()[0].as_ptr();
    let second_vtable = <TwinB as cppvtable_com::ComImplement>::vtable_slots()[0].as_ptr();
    assert!(
        !core::ptr::eq(first_vtable, second_vtable),
        "the two static vtables must keep two addresses"
    );

    assert!(first.as_impl::<TwinA>().is_some());
    assert!(first.as_impl::<TwinB>().is_none());
    assert!(second.as_impl::<TwinB>().is_some());
    assert!(second.as_impl::<TwinA>().is_none());

    assert_eq!(first.GammaValue(), 7);
    assert_eq!(second.GammaValue(), 8);
}

#[test]
fn a_secondary_chain_gives_the_same_object_to_a_c_caller() {
    let object = new_multi();
    let beta = object.cast::<IBeta>().unwrap();
    let this = beta.as_raw();

    // SAFETY: `this` is a valid interface pointer of `IBeta`.
    let vtable = unsafe { *this.cast::<*const IBetaVtbl>() };
    // SAFETY: The vtable belongs to the object.
    assert_eq!(unsafe { ((*vtable).BetaValue)(this) }, 31);
    // SAFETY: The test owns a public reference of the object.
    assert_eq!(unsafe { ((*vtable).base.AddRef)(this) }, 3);
    // SAFETY: This call removes the reference of the line above.
    assert_eq!(unsafe { ((*vtable).base.Release)(this) }, 2);
    assert_eq!(IBeta::NAME, "IBeta");
}
