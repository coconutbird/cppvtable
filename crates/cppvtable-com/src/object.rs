//! The object model: the allocation, the vtable pointers, and `QueryInterface`.
//!
//! [`ComObject<T>`] is the allocation of an object. The layout is:
//!
//! ```text
//! +---------------------+  <- the interface pointer of the primary interface
//! | vtable pointer 0    |
//! | vtable pointer 1    |  <- the interface pointer of the second interface chain
//! | ...                 |
//! +---------------------+
//! | the reference counts|
//! +---------------------+
//! | T (the Rust value)  |
//! +---------------------+
//! ```
//!
//! The structure is `#[repr(C)]`, so the vtable pointers are at the start and their
//! offsets are fixed. A shim of the macro `#[implement]` knows the index of its own
//! vtable pointer. It moves the `this` pointer back to the start of the allocation and
//! then reads the Rust value. This is the `this` adjustment of a C++ object that has
//! more than one base class.

use alloc::boxed::Box;
use core::ffi::c_void;
use core::mem::{offset_of, size_of};
use core::ptr::NonNull;

use crate::hresult::{E_NOINTERFACE, E_POINTER, HRESULT, S_OK};
use crate::interface::{ComInterface, IUnknownVtbl};
use crate::ptr::ComPtr;
use crate::refcount::{ForwardRefCount, RefCountPolicy, RefCounted, StandalonePolicy};
use crate::{GUID, VtablePtr};

/// The counts of the policy of `T`.
type StateOf<T> = <<T as RefCounted>::Policy as RefCountPolicy>::State;

/// The description of an implementation type.
///
/// The macro `#[implement]` makes the implementation. Do not write one by hand.
///
/// # Safety
///
/// The implementer must obey these rules:
///
/// - `Vtables` is `[VtablePtr; N]` with one entry for each implemented interface chain.
/// - Entry 0 belongs to `Primary`.
/// - `vtables` gives the address of a static vtable for each entry, and
///   `vtable_slots` gives the same addresses.
/// - `slot_for_iid` gives the index of the first interface chain that answers the
///   interface identifier, and it answers the IID of `IUnknown` with index 0.
/// - Its vtable shims receive interface pointers into a live [`ComObject`] of this type.
pub unsafe trait ComImplement: RefCounted {
    /// The array of the vtable pointers of the object.
    type Vtables: Copy + Send + Sync + 'static;

    /// The interface of the vtable pointer with the index 0.
    type Primary: ComInterface;

    /// Give the vtable pointers of a new object.
    fn vtables() -> Self::Vtables;

    /// Give the addresses of the static vtables of the type.
    fn vtable_slots() -> &'static [VtablePtr];

    /// Give the index of the interface chain that answers the interface identifier.
    fn slot_for_iid(iid: &GUID) -> Option<usize>;
}

/// The type `Self` implements the interface `I` at the vtable pointer with the index
/// [`Implements::SLOT`].
///
/// The macro `#[implement]` makes one implementation for each listed interface.
///
/// # Safety
///
/// `SLOT` must be the index of a vtable pointer of `Self` whose static vtable has the
/// layout of `I::Vtbl`.
pub unsafe trait Implements<I: ComInterface>: ComImplement {
    /// The index of the vtable pointer of the interface.
    const SLOT: usize;
}

/// The allocation of an object.
///
/// See the module documentation for the layout.
#[repr(C)]
pub struct ComObject<T: ComImplement> {
    /// One vtable pointer for each implemented interface chain.
    vtables: T::Vtables,
    /// The counts of the reference count policy.
    state: StateOf<T>,
    /// The Rust value.
    data: T,
}

impl<T: ComImplement> ComObject<T> {
    /// Give the Rust value.
    #[inline]
    #[must_use]
    pub fn data(&self) -> &T {
        &self.data
    }

    /// Give the counts of the reference count policy.
    #[inline]
    #[must_use]
    pub fn state(&self) -> &StateOf<T> {
        &self.state
    }

    /// Give the byte offset of the vtable pointer with the index `slot`.
    #[inline]
    #[must_use]
    pub const fn slot_offset(slot: usize) -> usize {
        offset_of!(Self, vtables) + slot * size_of::<VtablePtr>()
    }

    /// Give the interface pointer of the vtable pointer with the index `slot`.
    ///
    /// # Safety
    ///
    /// `object` must refer to a live `ComObject<T>`, and `slot` must be a valid index.
    #[inline]
    #[must_use]
    pub unsafe fn slot_ptr(object: *const Self, slot: usize) -> *mut c_void {
        // SAFETY: The caller gives a live object and a valid index, so the address is
        // inside the allocation.
        unsafe {
            object
                .cast::<u8>()
                .add(Self::slot_offset(slot))
                .cast::<c_void>()
                .cast_mut()
        }
    }

    /// Give the object of an interface pointer.
    ///
    /// This is the `this` adjustment of the shims.
    ///
    /// # Safety
    ///
    /// `this` must be the vtable pointer with the index `slot` of a live
    /// `ComObject<T>`.
    #[inline]
    #[must_use]
    pub unsafe fn from_slot<'a>(this: *mut c_void, slot: usize) -> &'a Self {
        // SAFETY: The caller gives the address of the field `vtables[slot]`. The offset
        // of that field is `slot_offset(slot)`, so the subtraction gives the start of
        // the allocation.
        unsafe {
            &*this
                .cast::<u8>()
                .sub(Self::slot_offset(slot))
                .cast::<Self>()
        }
    }

    /// Give the Rust value of an interface pointer.
    ///
    /// # Safety
    ///
    /// See [`ComObject::from_slot`].
    #[inline]
    #[must_use]
    pub unsafe fn impl_from_slot<'a>(this: *mut c_void, slot: usize) -> &'a T {
        // SAFETY: The caller obeys the rules of `from_slot`.
        unsafe { Self::from_slot(this, slot) }.data()
    }

    /// Give the object of a Rust value.
    ///
    /// # Safety
    ///
    /// `data` must be the field `data` of a live `ComObject<T>`. The generated vtable
    /// shims supply embedded values. Direct Rust calls can also use standalone values,
    /// which must not be passed to this function.
    #[inline]
    #[must_use]
    pub unsafe fn of_data(data: &T) -> *const Self {
        // SAFETY: The caller gives the field `data` of a `ComObject<T>`.
        unsafe {
            core::ptr::from_ref(data)
                .cast::<u8>()
                .sub(offset_of!(Self, data))
                .cast::<Self>()
        }
    }

    /// Destroy the object.
    ///
    /// This is the only destruction site of the crate. It runs the `Drop` of `T` and
    /// frees the allocation.
    ///
    /// # Safety
    ///
    /// The caller must own the last reference of the object. No other reference may
    /// exist, and nobody may use the object after the call.
    pub unsafe fn destroy(object: *const Self) {
        // SAFETY: `ComObject::new` and `OwnedObject::new` make the allocation with
        // `Box`. The caller owns the last reference, so this is the only `Box` that the
        // pointer becomes.
        drop(unsafe { Box::from_raw(object.cast_mut()) });
    }

    /// Make the allocation with the vtable pointers and the counts of the policy.
    fn allocate(value: T) -> *const Self {
        let object = Box::new(Self {
            vtables: T::vtables(),
            state: <T::Policy as RefCountPolicy>::new_state(),
            data: value,
        });
        Box::into_raw(object).cast_const()
    }
}

impl<T> ComObject<T>
where
    T: ComImplement,
    T::Primary: ComInterface,
    T::Policy: StandalonePolicy,
{
    /// Allocate the object and give one public reference of the primary interface.
    ///
    /// The public count goes from 0 to 1, so
    /// [`RefCounted::on_first_public_ref`] fires one time.
    #[expect(
        clippy::new_ret_no_self,
        reason = "The caller must get an owning reference, not the allocation."
    )]
    pub fn new(value: T) -> ComPtr<T::Primary> {
        let object = Self::allocate(value);
        // SAFETY: The object is new. This call owns the allocation, so it may add the
        // first public reference.
        unsafe { <T::Policy as RefCountPolicy>::add_ref(object) };
        // SAFETY: The object is live and the index 0 is the primary interface.
        let pointer = unsafe { Self::slot_ptr(object, 0) };
        // SAFETY: The pointer is the interface pointer of the primary interface, and
        // this call owns the public reference that `add_ref` made.
        unsafe { ComPtr::from_raw_unchecked(pointer) }
    }
}

/// An owning handle of an object that does not own itself.
///
/// Use it for a child object with [`ForwardRefCount`]. The container holds the handle.
/// The child is destroyed when the `Drop` of the container type drops the handle. This
/// gives the destruction order of Direct3D 9: the container destroys its surfaces and
/// volumes with itself.
///
pub struct OwnedObject<T>
where
    T: ComImplement + RefCounted<Policy = ForwardRefCount>,
{
    /// The allocation.
    object: NonNull<ComObject<T>>,
}

impl<T> OwnedObject<T>
where
    T: ComImplement + RefCounted<Policy = ForwardRefCount>,
{
    /// Allocate the object. The handle owns it.
    ///
    /// The call does not change the count of the container.
    #[must_use]
    pub fn new(value: T) -> Self {
        let object = ComObject::<T>::allocate(value);
        Self {
            // SAFETY: `Box::into_raw` never gives a null pointer.
            object: unsafe { NonNull::new_unchecked(object.cast_mut()) },
        }
    }

    /// Give the Rust value.
    #[inline]
    #[must_use]
    pub fn get(&self) -> &T {
        // SAFETY: The handle owns a live object.
        unsafe { self.object.as_ref() }.data()
    }

    /// Give the interface pointer of `I` without a change of a count.
    #[inline]
    #[must_use]
    pub fn as_raw<I: ComInterface>(&self) -> *mut c_void
    where
        T: Implements<I>,
    {
        // SAFETY: The handle owns a live object, and `Implements` gives a valid index.
        unsafe { ComObject::<T>::slot_ptr(self.object.as_ptr(), <T as Implements<I>>::SLOT) }
    }

    /// Give a public reference of `I`.
    ///
    /// The call adds one public reference. With [`ForwardRefCount`] the reference goes
    /// to the container. Use this for `GetSurfaceLevel` and equivalent methods.
    ///
    /// # Safety
    ///
    /// The container must own this handle and keep it alive until every public reference
    /// of the child has been released. Dropping the handle independently invalidates
    /// those references even when the container's count is positive.
    #[must_use]
    pub unsafe fn to_public<I: ComInterface>(&self) -> ComPtr<I>
    where
        T: Implements<I>,
    {
        // SAFETY: The handle owns a live object.
        unsafe { ForwardRefCount::add_ref(self.object.as_ptr().cast_const()) };
        // SAFETY: The call above added the public reference that the `ComPtr` owns.
        unsafe { ComPtr::from_raw_unchecked(self.as_raw::<I>()) }
    }
}

impl<T> core::ops::Deref for OwnedObject<T>
where
    T: ComImplement + RefCounted<Policy = ForwardRefCount>,
{
    type Target = T;

    fn deref(&self) -> &T {
        self.get()
    }
}

impl<T> Drop for OwnedObject<T>
where
    T: ComImplement + RefCounted<Policy = ForwardRefCount>,
{
    fn drop(&mut self) {
        // SAFETY: The handle owns the object, and the handle goes away now.
        unsafe { ComObject::<T>::destroy(self.object.as_ptr().cast_const()) };
    }
}

// SAFETY: The handle is an owning pointer. It may move to another thread when the value
// may move.
unsafe impl<T> Send for OwnedObject<T> where
    T: ComImplement + RefCounted<Policy = ForwardRefCount> + Send + Sync
{
}

// SAFETY: A shared reference of the handle gives a shared reference of the value.
unsafe impl<T> Sync for OwnedObject<T> where
    T: ComImplement + RefCounted<Policy = ForwardRefCount> + Send + Sync
{
}

/// Give the interface pointer of `I` for the object of a Rust value.
///
/// The call does not change a count. Use it inside a method of the implementation when
/// the object must give a pointer to itself, for example for `GetContainer`.
///
/// # Safety
///
/// `data` must be the value inside a live [`ComObject<T>`]. A generated vtable shim
/// supplies such a value, but a direct implementation-method call may use a standalone
/// Rust value. A method calling this function must require the embedded allocation in
/// its unsafe contract.
#[inline]
#[must_use]
pub unsafe fn interface_of<T, I>(data: &T) -> *mut c_void
where
    T: Implements<I>,
    I: ComInterface,
{
    // SAFETY: The caller gives the value inside a `ComObject<T>`.
    let object = unsafe { ComObject::<T>::of_data(data) };
    // SAFETY: The object is live and `Implements` gives a valid index.
    unsafe { ComObject::<T>::slot_ptr(object, <T as Implements<I>>::SLOT) }
}

/// The generated answer of `QueryInterface`.
///
/// The function answers the interface identifier of each implemented interface and of
/// each ancestor, and the identifier of `IUnknown`. It then calls
/// [`RefCounted::query_extra`].
///
/// # Safety
///
/// `object` must refer to a live `ComObject<T>`. `riid` and `out` come from the caller
/// of the COM method and may be null.
pub unsafe fn query_interface<T: ComImplement>(
    object: *const ComObject<T>,
    riid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: `out` is not null. The caller of a COM method gives a writable place.
    unsafe { *out = core::ptr::null_mut() };
    if riid.is_null() {
        return E_POINTER;
    }
    // SAFETY: `riid` is not null and refers to a GUID of the caller.
    let iid = unsafe { &*riid };

    if let Some(slot) = T::slot_for_iid(iid) {
        // SAFETY: The object is live and `slot_for_iid` gives a valid index.
        let pointer = unsafe { ComObject::<T>::slot_ptr(object, slot) };
        // SAFETY: The caller of `QueryInterface` owns a reference of the object.
        unsafe { <T::Policy as RefCountPolicy>::add_ref(object) };
        // SAFETY: `out` is not null.
        unsafe { *out = pointer };
        return S_OK;
    }

    // SAFETY: The object is live.
    if let Some(pointer) = unsafe { (*object).data().query_extra(iid) } {
        // SAFETY: `out` is not null.
        unsafe { *out = pointer.as_ptr() };
        return S_OK;
    }

    E_NOINTERFACE
}

impl IUnknownVtbl {
    /// Make the `IUnknown` part of a vtable of `T`.
    ///
    /// `SLOT` is the index of the vtable pointer that holds this vtable. The shims use
    /// the index for the `this` adjustment.
    #[must_use]
    pub const fn new<T: ComImplement, const SLOT: usize>() -> Self {
        Self {
            QueryInterface: unknown_query_interface::<T, SLOT>,
            AddRef: unknown_add_ref::<T, SLOT>,
            Release: unknown_release::<T, SLOT>,
        }
    }
}

/// The shim of `IUnknown::QueryInterface`.
unsafe extern "system" fn unknown_query_interface<T: ComImplement, const SLOT: usize>(
    this: *mut c_void,
    riid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    // SAFETY: A COM caller gives the interface pointer of the vtable with the index
    // `SLOT` of a live object.
    let object = unsafe { ComObject::<T>::from_slot(this, SLOT) };
    // SAFETY: The object is live.
    unsafe { query_interface::<T>(core::ptr::from_ref(object), riid, out) }
}

/// The shim of `IUnknown::AddRef`.
unsafe extern "system" fn unknown_add_ref<T: ComImplement, const SLOT: usize>(
    this: *mut c_void,
) -> u32 {
    // SAFETY: A COM caller gives the interface pointer of the vtable with the index
    // `SLOT` of a live object.
    let object = unsafe { ComObject::<T>::from_slot(this, SLOT) };
    // SAFETY: The caller owns a reference of the object.
    unsafe { <T::Policy as RefCountPolicy>::add_ref(core::ptr::from_ref(object)) }
}

/// The shim of `IUnknown::Release`.
unsafe extern "system" fn unknown_release<T: ComImplement, const SLOT: usize>(
    this: *mut c_void,
) -> u32 {
    // SAFETY: A COM caller gives the interface pointer of the vtable with the index
    // `SLOT` of a live object.
    let object = unsafe { ComObject::<T>::from_slot(this, SLOT) };
    // SAFETY: The caller owns the public reference that this call removes.
    unsafe { <T::Policy as RefCountPolicy>::release(core::ptr::from_ref(object)) }
}
