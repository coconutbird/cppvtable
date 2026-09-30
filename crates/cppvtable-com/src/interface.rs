//! COM interface metadata and the `IUnknown` interface.

use core::ffi::c_void;

use crate::hresult::HRESULT;
use crate::{GUID, Interface};

/// A binary interface with COM's `IUnknown` and `QueryInterface` semantics.
///
/// The `#[interface(abi = com)]` macro implements this marker and generates the `IUnknown`
/// vtable prefix. Do not implement it by hand.
///
/// # Safety
///
/// Its `IID` and `ANCESTORS` must match the COM interface chain and vtable prefix.
/// Every vtable must start with the three `IUnknown` methods.
pub unsafe trait ComInterface: Interface {
    /// The identifier used by `QueryInterface` for this interface.
    const IID: GUID;

    /// The identifiers of its bases, from the direct base to `IUnknown`.
    const ANCESTORS: &'static [GUID];
}

/// An interface whose objects can be used and released from any thread.
///
/// Implement this explicitly for an interface with a free-threaded contract. Atomic
/// reference counts alone do not establish that contract.
///
/// ```
/// use cppvtable_com::{AgileInterface, ComPtr, interface};
///
/// #[interface(abi = com, iid = "cb41b9d6-c106-4351-8d58-b5a5a2d3a631")]
/// unsafe trait IAgileValue {
///     fn Value(&self) -> u32;
/// }
///
/// // SAFETY: Every object exposing this interface permits concurrent calls and
/// // destruction from any thread.
/// unsafe impl AgileInterface for IAgileValue {}
/// fn require_send_sync<T: Send + Sync>() {}
/// require_send_sync::<ComPtr<IAgileValue>>();
/// ```
///
/// # Safety
///
/// Every object exposed through this interface must permit concurrent calls, reference
/// counting, and destruction on arbitrary threads without apartment restrictions. Rust
/// implementation types must implement `Send + Sync`, including the data returned by
/// [`crate::ComPtr::as_impl`], and their hooks must satisfy the same thread contract.
pub unsafe trait AgileInterface: ComInterface {}

/// Tell if an interface answers the interface identifier.
///
/// The answer is true for the IID of the interface and for the IID of each ancestor.
#[must_use]
pub fn interface_matches<I: ComInterface>(iid: &GUID) -> bool {
    *iid == I::IID || I::ANCESTORS.contains(iid)
}

/// The vtable of a COM object, seen as the vtable of `IUnknown`.
///
/// Every COM interface pointer refers to an object whose first three vtable slots are
/// the three methods of `IUnknown`.
///
/// # Safety
///
/// `ptr` must be a valid COM interface pointer.
#[inline]
pub unsafe fn unknown_add_ref(ptr: *mut c_void) -> u32 {
    // SAFETY: The caller gives a valid COM interface pointer. Slot 1 is `AddRef`.
    unsafe {
        let vtbl = *ptr.cast::<*const IUnknownVtbl>();
        ((*vtbl).AddRef)(ptr)
    }
}

/// Call `Release` through the vtable of a COM object.
///
/// # Safety
///
/// `ptr` must be a valid COM interface pointer, and the caller must own one public
/// reference.
#[inline]
pub unsafe fn unknown_release(ptr: *mut c_void) -> u32 {
    // SAFETY: The caller gives a valid COM interface pointer. Slot 2 is `Release`.
    unsafe {
        let vtbl = *ptr.cast::<*const IUnknownVtbl>();
        ((*vtbl).Release)(ptr)
    }
}

/// The root interface of COM.
///
/// The crate declares `IUnknown` with the same macro that a user crate uses. The option
/// `root` stops the macro from making the trait `IUnknownImpl` and the vtable builder.
/// The object model of this crate supplies both, because every object answers these
/// three methods in the same way.
#[cppvtable_macro::interface(
    abi = com,
    iid = "00000000-0000-0000-C000-000000000046",
    root,
    internal
)]
pub unsafe trait IUnknown {
    /// Ask the object for another interface.
    ///
    /// The object writes the interface pointer to `out` and adds one public reference.
    /// It returns `E_NOINTERFACE` and writes a null pointer when it does not have the
    /// interface.
    ///
    /// # Safety
    ///
    /// `riid` must be null or aligned and readable for one `GUID`. `out` must be null
    /// or aligned and writable for one pointer. Null arguments return `E_POINTER`.
    unsafe fn QueryInterface(&self, riid: *const GUID, out: *mut *mut c_void) -> HRESULT;

    /// Add one public reference. The method returns the new public count.
    ///
    /// # Safety
    ///
    /// The caller must keep the object alive with an existing reference during the call.
    unsafe fn AddRef(&self) -> u32;

    /// Remove one public reference. The method returns the new public count.
    ///
    /// # Safety
    ///
    /// The caller must own the reference being released. The object may be destroyed;
    /// references to this interface must not be used afterward unless independently owned.
    /// An owning Rust handle must relinquish that reference before this call so its
    /// destructor does not release it again.
    unsafe fn Release(&self) -> u32;
}
