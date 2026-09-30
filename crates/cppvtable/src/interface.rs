//! The `Interface` trait and the `IUnknown` interface.
//!
//! An interface type is a marker. It is a transparent wrapper of one pointer. The
//! pointer refers to a foreign object. The first field of that object is a pointer to
//! the vtable of the interface. This is the binary layout of a COM interface and of a
//! C++ class that has virtual methods.
//!
//! The macro `#[interface]` makes the interface type, the vtable structure, and the
//! trait `Interface`. Do not write an implementation of `Interface` by hand.

use core::ffi::c_void;

use crate::guid::GUID;
use crate::hresult::HRESULT;

/// The address of a static vtable.
///
/// The crate compares this address with the vtable pointer of an object. The comparison
/// tells if this process made the object. See [`crate::ComPtr::as_impl`].
#[repr(transparent)]
#[derive(Clone, Copy, Debug)]
pub struct VtablePtr(*const c_void);

// SAFETY: The value is the address of an immutable static. A thread can read the address
// and can compare it at any time.
unsafe impl Send for VtablePtr {}
// SAFETY: See the implementation of `Send`.
unsafe impl Sync for VtablePtr {}

impl VtablePtr {
    /// Make a `VtablePtr` from the address of a static vtable.
    #[inline]
    #[must_use]
    pub const fn new(address: *const c_void) -> Self {
        Self(address)
    }

    /// Give the address.
    #[inline]
    #[must_use]
    pub const fn as_ptr(self) -> *const c_void {
        self.0
    }

    /// Tell if the two addresses are the same.
    #[inline]
    #[must_use]
    pub fn is(self, address: *const c_void) -> bool {
        core::ptr::eq(self.0, address)
    }
}

/// The description of a binary interface.
///
/// `#[interface]` makes an implementation for each interface type.
///
/// # Safety
///
/// Do not implement this trait by hand. The implementer must obey these rules:
///
/// - `Self` is `#[repr(transparent)]` and holds exactly one `NonNull<c_void>`.
/// - That pointer is a valid interface pointer. Its first field is a pointer to
///   `Self::Vtbl`.
/// - `Vtbl` is `#[repr(C)]`. The first field is the vtable of the base interface.
/// - `ANCESTORS` holds the IID of each base interface, from the direct base to the root.
pub unsafe trait Interface: Sized + 'static {
    /// The vtable structure of the interface.
    type Vtbl: Sized + 'static;

    /// The interface identifier. A `cpp` or `c` interface without an `iid` uses the zero
    /// GUID.
    const IID: GUID;

    /// The IID of each base interface, from the direct base to the root.
    const ANCESTORS: &'static [GUID];

    /// True when the interface comes from `IUnknown` and answers `QueryInterface`.
    const IS_COM: bool;

    /// The name of the interface. Use it for log messages.
    const NAME: &'static str;
}

/// Give the raw interface pointer of an interface reference.
#[inline]
#[must_use]
pub fn raw_of<I: Interface>(this: &I) -> *mut c_void {
    // SAFETY: `Interface` requires that `I` is a transparent wrapper of one non-null
    // pointer. A read of that pointer is a read of a field of `this`.
    unsafe { *core::ptr::from_ref(this).cast::<*mut c_void>() }
}

/// Give the vtable pointer of an interface reference.
#[inline]
#[must_use]
pub fn vtable_of<I: Interface>(this: &I) -> *const I::Vtbl {
    // SAFETY: `Interface` requires that the pointer refers to an object whose first
    // field is a pointer to `I::Vtbl`.
    unsafe { *raw_of(this).cast::<*const I::Vtbl>() }
}

/// Tell if an interface answers the interface identifier.
///
/// The answer is true for the IID of the interface and for the IID of each ancestor. The
/// answer is always false for an interface that is not a COM interface.
#[must_use]
pub fn interface_matches<I: Interface>(iid: &GUID) -> bool {
    I::IS_COM && (*iid == I::IID || I::ANCESTORS.contains(iid))
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
    fn QueryInterface(&self, riid: *const GUID, out: *mut *mut c_void) -> HRESULT;

    /// Add one public reference. The method returns the new public count.
    fn AddRef(&self) -> u32;

    /// Remove one public reference. The method returns the new public count.
    fn Release(&self) -> u32;
}
