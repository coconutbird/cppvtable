//! COM behaviour: the vtable layout, `QueryInterface`, the counts, and the identity
//! rule.
//!
//! The tests call the object the way a C caller calls it: they read the vtable pointer
//! from the first field of the object and then call through the function pointer.

use core::ffi::c_void;
use core::mem::{offset_of, size_of};
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use cppvtable_com::{ComInterface, GUID, Interface};
use cppvtable_com::{
    ComObject, ComPtr, E_NOINTERFACE, E_POINTER, HRESULT, IUnknown, IUnknownVtbl, RefCounted, S_OK,
    SingleRefCount, implement, interface,
};

/// A counter interface.
#[interface(abi = com, iid = "0a1b2c3d-0001-4000-8000-000000000001")]
pub unsafe trait ICounter {
    /// Write the current value to `value`.
    ///
    /// # Safety
    ///
    /// `value` must be null or aligned and writable for one `u32`. Null returns `E_POINTER`.
    unsafe fn GetValue(&self, value: *mut u32) -> HRESULT;
    /// Add one to the value and give the new value.
    fn Increment(&self) -> u32;
}

/// A second interface of the same object. It is not in the chain of `ICounter`.
#[interface(abi = com, iid = "0a1b2c3d-0002-4000-8000-000000000002")]
pub unsafe trait INamed {
    /// Write the address of the name to `name`.
    ///
    /// # Safety
    ///
    /// `name` must be null or aligned and writable for one pointer. Null returns `E_POINTER`.
    unsafe fn GetName(&self, name: *mut *const u8) -> HRESULT;
}

/// An object that implements both interfaces.
///
/// Scalar implementation methods work directly on this Rust value. Methods that write
/// through caller pointers have explicit unsafe contracts regardless of visibility.
#[implement(ICounter, INamed)]
struct Counter {
    /// The value of the counter.
    value: AtomicU32,
    /// The test counts the destructions here.
    drops: Arc<AtomicU32>,
}

// SAFETY: Hooks obey the reference-count contract and all returned pointers stay live.
unsafe impl RefCounted for Counter {
    type Policy = SingleRefCount;
}

impl Drop for Counter {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

impl ICounterImpl for Counter {
    unsafe fn GetValue(&self, value: *mut u32) -> HRESULT {
        if value.is_null() {
            return E_POINTER;
        }
        // SAFETY: The pointer is not null, and the caller gives a writable place.
        unsafe { *value = self.value.load(Ordering::Relaxed) };
        S_OK
    }

    fn Increment(&self) -> u32 {
        self.value.fetch_add(1, Ordering::Relaxed) + 1
    }
}

impl INamedImpl for Counter {
    unsafe fn GetName(&self, name: *mut *const u8) -> HRESULT {
        if name.is_null() {
            return E_POINTER;
        }
        // SAFETY: The pointer is not null, and the caller gives a writable place.
        unsafe { *name = c"counter".as_ptr().cast::<u8>() };
        S_OK
    }
}

/// Make a new object and give the reference and the drop counter.
fn new_counter() -> (ComPtr<ICounter>, Arc<AtomicU32>) {
    let drops = Arc::new(AtomicU32::new(0));
    let object = ComObject::new(Counter {
        value: AtomicU32::new(10),
        drops: Arc::clone(&drops),
    });
    (object, drops)
}

/// Call `QueryInterface` the way a C caller does.
unsafe fn raw_query(this: *mut c_void, iid: &GUID) -> (HRESULT, *mut c_void) {
    let mut out: *mut c_void = ptr::null_mut();
    // SAFETY: `this` is a valid COM interface pointer, so its first field is the vtable
    // and slot 0 of that vtable is `QueryInterface`.
    let result = unsafe {
        let vtable = *this.cast::<*const IUnknownVtbl>();
        ((*vtable).QueryInterface)(this, ptr::from_ref(iid), &raw mut out)
    };
    (result, out)
}

#[test]
fn the_vtable_has_the_layout_of_a_com_vtable() {
    assert_eq!(size_of::<IUnknownVtbl>(), 3 * size_of::<usize>());
    assert_eq!(offset_of!(IUnknownVtbl, QueryInterface), 0);
    assert_eq!(offset_of!(IUnknownVtbl, AddRef), size_of::<usize>());
    assert_eq!(offset_of!(IUnknownVtbl, Release), 2 * size_of::<usize>());

    assert_eq!(size_of::<ICounterVtbl>(), 5 * size_of::<usize>());
    assert_eq!(offset_of!(ICounterVtbl, base), 0);
    assert_eq!(offset_of!(ICounterVtbl, GetValue), 3 * size_of::<usize>());
    assert_eq!(offset_of!(ICounterVtbl, Increment), 4 * size_of::<usize>());
}

#[test]
fn the_metadata_of_the_interface_is_correct() {
    assert_eq!(ICounter::NAME, "ICounter");
    // A COM interface answers its own IID and the IID of each ancestor.
    assert!(cppvtable_com::interface_matches::<ICounter>(&ICounter::IID));
    assert!(cppvtable_com::interface_matches::<ICounter>(&IUnknown::IID));
    assert!(!cppvtable_com::interface_matches::<ICounter>(&INamed::IID));
    assert_eq!(ICounter::ANCESTORS, &[IUnknown::IID]);
    assert_eq!(IUnknown::ANCESTORS, &[] as &[GUID]);
    assert_eq!(
        IUnknown::IID,
        GUID::from_values(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46])
    );
}

#[test]
fn a_c_caller_reaches_the_methods_through_the_vtable() {
    let (object, _drops) = new_counter();
    let this = object.as_raw();

    // SAFETY: `this` is a valid interface pointer of `ICounter`.
    let vtable = unsafe { *this.cast::<*const ICounterVtbl>() };
    let mut value = 0_u32;
    // SAFETY: The vtable is the vtable of the object and `value` is a local value.
    let result = unsafe { ((*vtable).GetValue)(this, &raw mut value) };
    assert!(result.is_ok());
    assert_eq!(value, 10);

    // SAFETY: The vtable is the vtable of the object.
    let next = unsafe { ((*vtable).Increment)(this) };
    assert_eq!(next, 11);

    // The same call through the interface wrapper gives the same answer.
    // SAFETY: The object is alive.
    let after = unsafe { object.Increment() };
    assert_eq!(after, 12);
}

#[test]
fn a_c_caller_reaches_the_second_interface_through_its_own_vtable() {
    let (object, _drops) = new_counter();
    let (result, raw) = {
        // SAFETY: The object is alive.
        unsafe { raw_query(object.as_raw(), &INamed::IID) }
    };
    assert!(result.is_ok());
    assert!(!raw.is_null());
    // The second interface pointer is not the first one. The `this` adjustment moved it.
    assert_ne!(raw, object.as_raw());
    assert_eq!(
        raw as usize - object.as_raw() as usize,
        size_of::<*const c_void>()
    );

    // SAFETY: `raw` is a valid interface pointer of `INamed`.
    let named = unsafe { ComPtr::<INamed>::from_raw(raw) }.unwrap();
    let mut name: *const u8 = ptr::null();
    // SAFETY: The object is alive and `name` is a local value.
    let result = unsafe { named.GetName(&raw mut name) };
    assert!(result.is_ok());
    // SAFETY: The method gives the address of a static C string.
    let text = unsafe { core::ffi::CStr::from_ptr(name.cast::<core::ffi::c_char>()) };
    assert_eq!(text.to_bytes(), b"counter");
}

#[test]
fn query_interface_answers_self_the_ancestor_and_the_second_interface() {
    let (object, _drops) = new_counter();
    let this = object.as_raw();

    for iid in [ICounter::IID, IUnknown::IID] {
        // SAFETY: `this` is a valid COM interface pointer.
        let (result, raw) = unsafe { raw_query(this, &iid) };
        assert!(result.is_ok());
        assert_eq!(raw, this);
        // SAFETY: `QueryInterface` added the reference that this `ComPtr` owns.
        drop(unsafe { ComPtr::<ICounter>::from_raw(raw) });
    }

    // SAFETY: `this` is a valid COM interface pointer.
    let (result, raw) = unsafe { raw_query(this, &INamed::IID) };
    assert!(result.is_ok());
    // SAFETY: `QueryInterface` added the reference that this `ComPtr` owns.
    drop(unsafe { ComPtr::<INamed>::from_raw(raw) });
}

#[test]
fn query_interface_refuses_an_unknown_interface_and_a_null_out_pointer() {
    let (object, _drops) = new_counter();
    let this = object.as_raw();

    let unknown_iid = GUID::from_values(0xdead_beef, 0, 0, [0; 8]);
    // SAFETY: `this` is a valid COM interface pointer.
    let (result, raw) = unsafe { raw_query(this, &unknown_iid) };
    assert_eq!(result, E_NOINTERFACE);
    assert!(raw.is_null());

    // SAFETY: `this` is a valid COM interface pointer. A null out-pointer is the case
    // that the test checks.
    let result = unsafe {
        let vtable = *this.cast::<*const IUnknownVtbl>();
        ((*vtable).QueryInterface)(this, &raw const unknown_iid, ptr::null_mut())
    };
    assert_eq!(result, E_POINTER);
}

#[test]
fn the_identity_rule_holds_from_every_interface() {
    let (object, _drops) = new_counter();
    let named = object.cast::<INamed>().unwrap();
    assert_ne!(named.as_raw(), object.as_raw());

    let from_counter = object.cast::<IUnknown>().unwrap();
    let from_named = named.cast::<IUnknown>().unwrap();
    assert_eq!(from_counter.as_raw(), from_named.as_raw());
    assert_eq!(from_counter.as_raw(), object.as_raw());
}

#[test]
fn the_counts_and_the_destruction_are_correct() {
    let (object, drops) = new_counter();
    let this = object.as_raw();
    // SAFETY: `this` is a valid COM interface pointer of a live object.
    let vtable = unsafe { *this.cast::<*const IUnknownVtbl>() };

    // SAFETY: The object is alive and this call owns a reference.
    assert_eq!(unsafe { ((*vtable).AddRef)(this) }, 2);
    // SAFETY: The object is alive and this call owns a reference.
    assert_eq!(unsafe { ((*vtable).AddRef)(this) }, 3);
    // SAFETY: This call removes one of the references that the test owns.
    assert_eq!(unsafe { ((*vtable).Release)(this) }, 2);
    // SAFETY: This call removes one of the references that the test owns.
    assert_eq!(unsafe { ((*vtable).Release)(this) }, 1);
    assert_eq!(drops.load(Ordering::Relaxed), 0);

    drop(object);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn a_clone_adds_a_reference_and_a_drop_removes_it() {
    let (object, drops) = new_counter();
    let second = object.clone();
    assert_eq!(second.public_count_of::<Counter>(), Some(2));
    drop(second);
    assert_eq!(object.public_count_of::<Counter>(), Some(1));
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    drop(object);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn as_impl_answers_only_for_an_object_of_this_process() {
    let (object, _drops) = new_counter();
    let value = object.as_impl::<Counter>().unwrap();
    assert_eq!(value.value.load(Ordering::Relaxed), 10);

    // A foreign object has a vtable that this process did not make.
    let foreign_vtable: *const c_void = ptr::from_ref(&FOREIGN_VTABLE).cast();
    let mut foreign_object = foreign_vtable;
    let raw: *mut c_void = ptr::from_mut(&mut foreign_object).cast();
    // SAFETY: `raw` refers to a place whose first field is a vtable pointer.
    let found = unsafe { cppvtable_com::object_of_raw::<Counter>(raw) };
    assert!(found.is_none());
}

/// A vtable that this process did not make. `as_impl` must not answer for it.
static FOREIGN_VTABLE: [usize; 8] = [0; 8];
