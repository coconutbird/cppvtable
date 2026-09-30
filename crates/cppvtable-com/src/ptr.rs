//! The two smart pointers: [`ComPtr`] and [`PrivateRef`], and the out-parameter helper
//! [`write_out`].
//!
//! [`ComPtr<I>`] owns one public reference of the interface `I`. It works with an object
//! of this process and with an object of the application.
//!
//! [`PrivateRef<T>`] owns one private reference of an object of this process. It needs
//! the concrete implementation type `T`, and the policy of `T` must have a private count
//! ([`crate::DualRefCount`]).

use core::ffi::c_void;
use core::fmt;
use core::marker::PhantomData;
use core::ops::Deref;
use core::ptr::NonNull;

use crate::hresult::{E_POINTER, HRESULT, S_OK};
use crate::interface::{AgileInterface, ComInterface, unknown_add_ref, unknown_release};
use crate::object::{ComImplement, ComObject, Implements};
use crate::refcount::{PrivatePolicy, RefCountPolicy};

/// Give the object of a raw interface pointer when this process made the object with
/// the type `T`.
///
/// The function reads the vtable pointer of the object and compares it with the address
/// of each static vtable of `T`. It reads nothing else, so a pointer to a foreign object
/// gives `None` and no read goes outside the object.
///
/// The test is correct because each `#[implement]` makes its own static for each vtable,
/// and two statics have two addresses. The compiler does not merge two Rust statics, and
/// the linker folds only COMDAT sections, which a plain static of a crate is not. The
/// test `two_types_with_the_same_shape_keep_two_vtables` of `tests/multiple.rs` checks
/// this with optimization on.
///
/// # Safety
///
/// `raw` must be null or a valid interface pointer. The first field of the object must
/// be its vtable pointer. Every COM interface pointer obeys this rule.
#[must_use]
pub unsafe fn object_of_raw<T: ComImplement>(raw: *mut c_void) -> Option<*const ComObject<T>> {
    let raw = NonNull::new(raw)?;
    // SAFETY: The caller gives a valid interface pointer. The first field of the object
    // is the vtable pointer.
    let vtable = unsafe { *raw.as_ptr().cast::<*const c_void>() };
    for (slot, known) in T::vtable_slots().iter().enumerate() {
        if known.is(vtable) {
            // SAFETY: The vtable pointer is the address of the static vtable of `T` for
            // the index `slot`. Only `ComObject<T>` holds that address, so the object is
            // a `ComObject<T>` and `raw` is its field `vtables[slot]`.
            let object = unsafe { ComObject::<T>::from_slot(raw.as_ptr(), slot) };
            return Some(core::ptr::from_ref(object));
        }
    }
    None
}

/// Write an interface pointer to a COM out-parameter and return the status.
///
/// Declare a COM out-parameter as `*mut Option<ComPtr<I>>`. `Option<ComPtr<I>>` has the
/// layout of one nullable interface pointer, so the parameter has the ABI of
/// `I** out` in C and C++, and a Rust caller can pass `&raw mut slot` for a local
/// `let mut slot = None;`.
///
/// The function writes with [`core::ptr::write`]. It never reads or drops the old
/// contents of the slot, which a foreign caller may leave uninitialized; an
/// assignment `*out = value` would `Release` that garbage.
///
/// A null `out` gives `E_POINTER` and drops `value`, which releases its reference.
/// Otherwise the slot takes the reference of `value` and the function gives `S_OK`,
/// also for `None`; return another code yourself when a null result is a failure.
///
/// # Safety
///
/// `out` must be null or aligned and writable for one `Option<ComPtr<I>>`.
pub unsafe fn write_out<I: ComInterface>(
    out: *mut Option<ComPtr<I>>,
    value: Option<ComPtr<I>>,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: `out` is not null, and the caller gives an aligned, writable place. The
    // write does not read the old contents.
    unsafe { out.write(value) };
    S_OK
}

/// An owning public reference of a COM interface `I`.
///
/// For COM interfaces, `Clone` calls `AddRef` and `Drop` calls `Release`. The type
/// derefs to `I`, so the methods of the interface and of each base interface are
/// available, including [`crate::IUnknown::cast`] for `QueryInterface`.
///
/// The type is `#[repr(transparent)]` over one non-null pointer, so
/// `Option<ComPtr<I>>` has the size of a pointer and the ABI of a nullable interface
/// pointer; see [`write_out`] for out-parameters.
///
/// Moving a pointer across threads requires an explicit [`AgileInterface`] contract.
/// `IUnknown` itself has no such contract:
///
/// ```compile_fail
/// use cppvtable_com::{ComPtr, IUnknown};
/// fn require_send<T: Send>() {}
/// require_send::<ComPtr<IUnknown>>();
/// ```
#[repr(transparent)]
pub struct ComPtr<I: ComInterface> {
    /// The interface pointer.
    ptr: NonNull<c_void>,
    /// The interface type.
    marker: PhantomData<I>,
}

// SAFETY: `AgileInterface` guarantees that its objects support use and destruction on
// any thread, including concurrent reference-count operations.
unsafe impl<I: AgileInterface> Send for ComPtr<I> {}
// SAFETY: See the implementation of `Send`.
unsafe impl<I: AgileInterface> Sync for ComPtr<I> {}

impl<I: ComInterface> ComPtr<I> {
    /// Take ownership of a raw interface pointer.
    ///
    /// The call does not add a reference. Use it for a pointer that a COM method gave
    /// to you, for example the out-parameter of `QueryInterface`.
    ///
    /// # Safety
    ///
    /// `raw` must not be null, it must be an interface pointer of `I`, and the caller
    /// must own one public reference of it.
    #[inline]
    #[must_use]
    pub const unsafe fn from_raw_unchecked(raw: *mut c_void) -> Self {
        Self {
            // SAFETY: The caller gives a pointer that is not null.
            ptr: unsafe { NonNull::new_unchecked(raw) },
            marker: PhantomData,
        }
    }

    /// Take ownership of a raw interface pointer. A null pointer gives `None`.
    ///
    /// # Safety
    ///
    /// See [`ComPtr::from_raw_unchecked`].
    #[inline]
    #[must_use]
    pub unsafe fn from_raw(raw: *mut c_void) -> Option<Self> {
        NonNull::new(raw).map(|ptr| Self {
            ptr,
            marker: PhantomData,
        })
    }

    /// Add one public reference of a raw interface pointer and own it.
    ///
    /// Use it for a pointer that the application gave as an argument. The application
    /// keeps its own reference. For a typed interface reference, use
    /// [`ComPtr::from_ref`].
    ///
    /// # Safety
    ///
    /// `raw` must be null or a valid interface pointer of `I`, and it must stay alive
    /// during the call.
    #[must_use]
    pub unsafe fn from_raw_add_ref(raw: *mut c_void) -> Option<Self> {
        let ptr = NonNull::new(raw)?;
        // SAFETY: The caller gives a valid COM interface pointer.
        unsafe { unknown_add_ref(ptr.as_ptr()) };
        Some(Self {
            ptr,
            marker: PhantomData,
        })
    }

    /// Add one public reference of a borrowed interface and own it.
    ///
    /// Use it for an interface that an argument or an [`crate::InterfaceRef`] borrows
    /// when the result must outlive the borrow.
    #[must_use]
    pub fn from_ref(iface: &I) -> Self {
        let ptr = cppvtable_abi::interface::raw_of(iface);
        // SAFETY: An interface value exists only for a live interface pointer of `I`,
        // and the borrow keeps the object alive during the call.
        unsafe { unknown_add_ref(ptr) };
        // SAFETY: The pointer of an interface value is not null, and the call above
        // added the public reference that the `ComPtr` owns.
        unsafe { Self::from_raw_unchecked(ptr) }
    }

    /// Add one public reference of the object of a Rust value and own it.
    ///
    /// Use it inside a method of the implementation when the object must give a public
    /// reference of itself. The public count goes up by one, so
    /// [`crate::RefCounted::on_first_public_ref`] fires when the count was zero.
    ///
    /// # Safety
    ///
    /// `data` must be the value inside a live [`ComObject<T>`]. A generated vtable shim
    /// supplies such a value, but a direct implementation-method call may use a
    /// standalone Rust value. Its method contract must require this embedded allocation
    /// when it calls `from_impl`.
    #[must_use]
    pub unsafe fn from_impl<T: Implements<I>>(data: &T) -> Self {
        // SAFETY: The caller gives the value inside a `ComObject<T>`.
        let object = unsafe { ComObject::<T>::of_data(data) };
        // SAFETY: The object is live, and the caller owns a reference of it.
        unsafe { <T::Policy as RefCountPolicy>::add_ref(object) };
        // SAFETY: The object is live and `Implements` gives a valid index.
        let pointer = unsafe { ComObject::<T>::slot_ptr(object, <T as Implements<I>>::SLOT) };
        // SAFETY: The call above added the public reference that the `ComPtr` owns.
        unsafe { Self::from_raw_unchecked(pointer) }
    }

    /// Give the raw interface pointer. The `ComPtr` keeps the reference.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> *mut c_void {
        self.ptr.as_ptr()
    }

    /// Give the raw interface pointer and give up the reference.
    ///
    /// Use it to fill an out-parameter of a COM method. The caller of the method then
    /// owns the reference.
    #[inline]
    #[must_use]
    pub fn into_raw(self) -> *mut c_void {
        let kept = core::mem::ManuallyDrop::new(self);
        kept.ptr.as_ptr()
    }

    /// Give the Rust value when this process made the object with the type `T`.
    ///
    /// The method compares the vtable pointer of the object with the addresses of the
    /// static vtables of `T`. An object of the application gives `None`.
    #[must_use]
    pub fn as_impl<T: ComImplement>(&self) -> Option<&T> {
        // SAFETY: The `ComPtr` holds a valid interface pointer.
        let object = unsafe { object_of_raw::<T>(self.ptr.as_ptr()) }?;
        // SAFETY: The object is live because this `ComPtr` owns a public reference.
        Some(unsafe { &*object }.data())
    }

    /// Give the public count when this process made the object with the type `T`.
    ///
    /// Use it for tests and for log messages.
    #[must_use]
    pub fn public_count_of<T: ComImplement>(&self) -> Option<u32> {
        // SAFETY: The `ComPtr` holds a valid interface pointer.
        let object = unsafe { object_of_raw::<T>(self.ptr.as_ptr()) }?;
        // SAFETY: The object is live because this `ComPtr` owns a public reference.
        Some(unsafe { <T::Policy as RefCountPolicy>::public_count(object) })
    }
}

impl<I: ComInterface> Clone for ComPtr<I> {
    fn clone(&self) -> Self {
        // SAFETY: The `ComPtr` holds a valid COM interface pointer of a live object.
        unsafe { unknown_add_ref(self.ptr.as_ptr()) };
        Self {
            ptr: self.ptr,
            marker: PhantomData,
        }
    }
}

impl<I: ComInterface> Drop for ComPtr<I> {
    fn drop(&mut self) {
        // SAFETY: The `ComPtr` owns the public reference that this call removes.
        unsafe { unknown_release(self.ptr.as_ptr()) };
    }
}

impl<I: ComInterface> Deref for ComPtr<I> {
    type Target = I;

    #[inline]
    fn deref(&self) -> &I {
        // SAFETY: `ComPtr<I>` and `I` are both transparent wrappers of one
        // `NonNull<c_void>`, so the two types have the same layout.
        unsafe { &*core::ptr::from_ref(self).cast::<I>() }
    }
}

impl<I: ComInterface> fmt::Debug for ComPtr<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ComPtr<{}>({:p})", I::NAME, self.ptr.as_ptr())
    }
}

impl<I: ComInterface> PartialEq for ComPtr<I> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
}

impl<I: ComInterface> Eq for ComPtr<I> {}

/// An owning private reference of an object of this process.
///
/// The private count belongs to the frontend. It keeps the object alive after the
/// application released it, but it does not keep the public count larger than zero. The
/// device of Direct3D 9 holds a `PrivateRef` of each bound resource.
///
/// The type derefs to `T`, so the Rust methods of the implementation are available
/// without a call through the vtable.
pub struct PrivateRef<T>
where
    T: ComImplement,
    T::Policy: PrivatePolicy,
{
    /// The allocation.
    object: NonNull<ComObject<T>>,
}

// SAFETY: The counts are atomic, and `T` decides if the value may move to another
// thread.
unsafe impl<T> Send for PrivateRef<T>
where
    T: ComImplement + Send + Sync,
    T::Policy: PrivatePolicy,
{
}

// SAFETY: See the implementation of `Send`.
unsafe impl<T> Sync for PrivateRef<T>
where
    T: ComImplement + Send + Sync,
    T::Policy: PrivatePolicy,
{
}

impl<T> PrivateRef<T>
where
    T: ComImplement,
    T::Policy: PrivatePolicy,
{
    /// Make a private reference of an object that this process made.
    ///
    /// Use it for an interface pointer that the application gives as an argument, for
    /// example the texture of `SetTexture`. The call adds one private reference, and the
    /// caller keeps its own reference. A pointer to a foreign object gives `None`.
    ///
    /// # Safety
    ///
    /// `raw` must be null or a valid interface pointer of a live object.
    #[must_use]
    pub unsafe fn from_raw_add_ref(raw: *mut c_void) -> Option<Self> {
        // SAFETY: The caller gives a valid interface pointer.
        let object = unsafe { object_of_raw::<T>(raw) }?;
        // SAFETY: The caller owns a reference of the object, so it is live.
        unsafe { <T::Policy as PrivatePolicy>::add_private(object) };
        Some(Self {
            // SAFETY: `object_of_raw` never gives a null pointer.
            object: unsafe { NonNull::new_unchecked(object.cast_mut()) },
        })
    }

    /// Make a private reference from a public reference.
    ///
    /// A pointer to a foreign object gives `None`.
    #[must_use]
    pub fn from_com_ptr<I: ComInterface>(pointer: &ComPtr<I>) -> Option<Self> {
        // SAFETY: A `ComPtr` holds a valid interface pointer of a live object.
        unsafe { Self::from_raw_add_ref(pointer.as_raw()) }
    }

    /// Make a private reference of the object of a Rust value.
    ///
    /// Use it inside a method of the implementation when the object must give a
    /// reference of itself.
    ///
    /// # Safety
    ///
    /// `data` must be the value inside a live [`ComObject<T>`]. A generated vtable shim
    /// supplies such a value, but a direct implementation-method call may use a
    /// standalone Rust value. Its method contract must require this embedded allocation
    /// when it calls `from_impl`.
    #[must_use]
    pub unsafe fn from_impl(data: &T) -> Self {
        // SAFETY: The caller gives the value inside a `ComObject<T>`.
        let object = unsafe { ComObject::<T>::of_data(data) };
        // SAFETY: The object is live.
        unsafe { <T::Policy as PrivatePolicy>::add_private(object) };
        Self {
            // SAFETY: `of_data` never gives a null pointer.
            object: unsafe { NonNull::new_unchecked(object.cast_mut()) },
        }
    }

    /// Give the interface pointer of `I` without a change of a count.
    #[inline]
    #[must_use]
    pub fn as_raw<I: ComInterface>(&self) -> *mut c_void
    where
        T: Implements<I>,
    {
        // SAFETY: The object is live and `Implements` gives a valid index.
        unsafe { ComObject::<T>::slot_ptr(self.object.as_ptr(), <T as Implements<I>>::SLOT) }
    }

    /// Add one public reference and give it.
    ///
    /// This is how `GetTexture` and equivalent methods answer. When the public count was
    /// zero, [`crate::RefCounted::on_first_public_ref`] fires again.
    #[must_use]
    pub fn to_public<I: ComInterface>(&self) -> ComPtr<I>
    where
        T: Implements<I>,
    {
        // SAFETY: The private reference keeps the object alive.
        unsafe { <T::Policy as RefCountPolicy>::add_ref(self.object.as_ptr().cast_const()) };
        // SAFETY: The call above added the public reference that the `ComPtr` owns.
        unsafe { ComPtr::from_raw_unchecked(self.as_raw::<I>()) }
    }

    /// Give the current public count. Use it for tests and for log messages.
    #[must_use]
    pub fn public_count(&self) -> u32 {
        // SAFETY: The private reference keeps the object alive.
        unsafe { <T::Policy as RefCountPolicy>::public_count(self.object.as_ptr().cast_const()) }
    }

    /// Give the current private count without the reference of the public count.
    #[must_use]
    pub fn private_count(&self) -> u32 {
        // SAFETY: The private reference keeps the object alive.
        unsafe { <T::Policy as PrivatePolicy>::private_count(self.object.as_ptr().cast_const()) }
    }

    /// Tell if the two references refer to the same object.
    #[inline]
    #[must_use]
    pub fn is(&self, other: &Self) -> bool {
        self.object == other.object
    }
}

impl<T> Clone for PrivateRef<T>
where
    T: ComImplement,
    T::Policy: PrivatePolicy,
{
    fn clone(&self) -> Self {
        // SAFETY: This reference keeps the object alive.
        unsafe { <T::Policy as PrivatePolicy>::add_private(self.object.as_ptr().cast_const()) };
        Self {
            object: self.object,
        }
    }
}

impl<T> Drop for PrivateRef<T>
where
    T: ComImplement,
    T::Policy: PrivatePolicy,
{
    fn drop(&mut self) {
        // SAFETY: This reference owns the private count that the call removes.
        unsafe { <T::Policy as PrivatePolicy>::release_private(self.object.as_ptr().cast_const()) };
    }
}

impl<T> Deref for PrivateRef<T>
where
    T: ComImplement,
    T::Policy: PrivatePolicy,
{
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: The private reference keeps the object alive.
        unsafe { self.object.as_ref() }.data()
    }
}

impl<T> fmt::Debug for PrivateRef<T>
where
    T: ComImplement,
    T::Policy: PrivatePolicy,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PrivateRef({:p})", self.object.as_ptr())
    }
}
