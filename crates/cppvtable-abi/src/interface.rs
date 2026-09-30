//! Generic interface-pointer metadata for C and C++ vtables.
//!
//! An interface value is a transparent wrapper of one pointer. The pointer refers to a
//! foreign object whose first field is a pointer to the interface vtable.

use core::ffi::c_void;

/// The address of a static vtable.
///
/// Object runtimes store these addresses in the interface slots of an allocation.
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
/// This metadata describes the pointer and vtable layout shared by C and C++ interfaces.
/// COM identifiers and inheritance metadata belong to `cppvtable-com::ComInterface`.
///
/// # Safety
///
/// Do not implement this trait by hand. The implementer must obey these rules:
///
/// - `Self` is `#[repr(transparent)]` and holds exactly one `NonNull<c_void>`.
/// - That pointer is a valid interface pointer. Its first field is a pointer to
///   `Self::Vtbl`.
/// - `Vtbl` is `#[repr(C)]`. For a derived interface its first field is the base vtable.
pub unsafe trait Interface: Sized + 'static {
    /// The vtable structure of the interface.
    type Vtbl: Sized + 'static;

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
