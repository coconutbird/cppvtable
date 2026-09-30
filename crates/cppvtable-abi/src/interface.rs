//! Generic interface-pointer metadata for C and C++ vtables.
//!
//! An interface value is a transparent wrapper of one pointer. The pointer refers to a
//! foreign object containing either a vtable pointer or the vtable entries themselves.
//! Safe code never owns an interface value: it borrows one through an
//! [`InterfaceRef`], a reference, or an owning runtime handle.

use core::ffi::c_void;
use core::fmt;
use core::marker::PhantomData;
use core::ops::Deref;
use core::ptr::NonNull;

/// Where an interface stores its function table relative to the interface pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VtableLayout {
    /// The interface starts with a pointer to a separate table.
    Pointer,
    /// The table entries begin directly at the interface pointer, with no indirection.
    Inline,
}

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

/// The pointer field of a generated interface type.
///
/// It is a transparent wrapper of one non-null interface pointer. It is not `Clone` or
/// `Copy` and has no constructor, so safe code can neither make an interface value nor
/// copy a borrowed one out of its borrow. Generated interface methods are safe to call,
/// and this is what keeps every interface value tied to the lifetime of its object.
#[repr(transparent)]
pub struct RawInterface(NonNull<c_void>);

impl RawInterface {
    /// Give the interface pointer.
    #[inline]
    #[must_use]
    pub const fn as_ptr(&self) -> *mut c_void {
        self.0.as_ptr()
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
/// - `Self` is `#[repr(transparent)]` and holds exactly one [`RawInterface`], so it is
///   neither `Clone` nor `Copy` and safe code cannot construct it.
/// - That pointer is a valid interface pointer with the representation selected by
///   `LAYOUT`: either its first field points to `Self::Vtbl`, or it points directly
///   to the inline `Self::Vtbl` prefix. The table must remain valid and immutable
///   while borrowed.
/// - `Vtbl` is `#[repr(C)]`. For a derived interface its first field is the base vtable.
pub unsafe trait Interface: Sized + 'static {
    /// The vtable structure of the interface.
    type Vtbl: Sized + 'static;

    /// The physical representation of the interface's function table.
    const LAYOUT: VtableLayout = VtableLayout::Pointer;

    /// C++ runtime ABI, if this is a C++ interface rather than a C or COM table.
    ///
    /// This selects the RTTI representation; it does not assert that any particular
    /// object's table actually contains RTTI. Inspecting such metadata remains unsafe.
    const CPP_ABI: Option<crate::rtti::CppAbi> = None;

    /// The name of the interface. Use it for log messages.
    const NAME: &'static str;
}

/// A borrowed interface pointer of the interface `I`, valid for the lifetime `'a`.
///
/// It is a copyable, pointer-sized handle that dereferences to `I`, so every method of
/// the interface is available on it. `Option<InterfaceRef<'_, I>>` is a nullable
/// interface pointer with the layout of `*mut c_void`, which makes it usable as a
/// borrowed in-parameter of a vtable method.
///
/// The debug output is `InterfaceRef<IName>(0x…)`, and equality compares the interface
/// pointers.
#[repr(transparent)]
pub struct InterfaceRef<'a, I: Interface> {
    raw: NonNull<c_void>,
    _borrow: PhantomData<&'a I>,
}

impl<I: Interface> InterfaceRef<'_, I> {
    /// Borrow a raw interface pointer. A null pointer gives `None`.
    ///
    /// # Safety
    ///
    /// A non-null `raw` must obey the contract of [`InterfaceRef::from_non_null`].
    #[inline]
    #[must_use]
    pub unsafe fn from_raw(raw: *mut c_void) -> Option<Self> {
        // SAFETY: The caller gives the contract of `from_non_null` for a non-null pointer.
        NonNull::new(raw).map(|raw| unsafe { Self::from_non_null(raw) })
    }

    /// Borrow a non-null raw interface pointer.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid interface pointer of `I` for the whole lifetime `'a`: the
    /// object must stay alive, its function table must stay valid and unmodified, and the
    /// object must obey the declaration of `I`.
    #[inline]
    #[must_use]
    pub unsafe fn from_non_null(raw: NonNull<c_void>) -> Self {
        const {
            assert!(
                size_of::<I>() == size_of::<NonNull<c_void>>()
                    && align_of::<I>() == align_of::<NonNull<c_void>>(),
                "an interface type must be a transparent wrapper of one pointer"
            );
        }
        Self {
            raw,
            _borrow: PhantomData,
        }
    }
}

impl<I: Interface> Deref for InterfaceRef<'_, I> {
    type Target = I;

    #[inline]
    fn deref(&self) -> &I {
        // SAFETY: `Interface` makes `I` a transparent wrapper of one non-null pointer, so
        // the field has the layout of `I`. The constructor guarantees that the pointer is
        // a valid interface pointer of `I` for `'a`, which outlives this borrow.
        unsafe { &*core::ptr::from_ref(&self.raw).cast::<I>() }
    }
}

impl<I: Interface> Clone for InterfaceRef<'_, I> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<I: Interface> Copy for InterfaceRef<'_, I> {}

impl<I: Interface> fmt::Debug for InterfaceRef<'_, I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InterfaceRef<{}>({:p})", I::NAME, self.raw)
    }
}

impl<I: Interface> PartialEq for InterfaceRef<'_, I> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<I: Interface> Eq for InterfaceRef<'_, I> {}

/// Give the raw interface pointer of an interface reference.
#[inline]
#[must_use]
pub fn raw_of<I: Interface>(this: &I) -> *mut c_void {
    // SAFETY: `Interface` requires that `I` is a transparent wrapper of one non-null
    // pointer. A read of that pointer is a read of a field of `this`.
    unsafe { *core::ptr::from_ref(this).cast::<*mut c_void>() }
}

/// Give the vtable of an interface reference.
///
/// The table is borrowed for as long as the interface reference.
#[inline]
#[must_use]
pub fn vtable_of<I: Interface>(this: &I) -> &I::Vtbl {
    match I::LAYOUT {
        VtableLayout::Pointer => {
            // SAFETY: This layout requires that the first field of the live object points
            // to an `I::Vtbl` that stays valid and immutable while borrowed.
            unsafe { &**raw_of(this).cast::<*const I::Vtbl>() }
        }
        VtableLayout::Inline => {
            // SAFETY: This layout requires that the object starts with an `I::Vtbl` that
            // stays valid and immutable while borrowed.
            unsafe { &*raw_of(this).cast::<I::Vtbl>() }
        }
    }
}
