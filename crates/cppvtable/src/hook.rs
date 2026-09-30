//! Heap-allocated vtable copies for RTTI classes and virtual-method hooking.
//!
//! Portable hooking is per object: copy the native table into a [`ShadowVtable`],
//! replace selected entries, and point one object at the copy with [`swap_vtable`].
//! Other objects of the class are unaffected, and type identity and casts keep
//! working because the native prefix is copied unchanged. [`patch_vtable_entry`]
//! offers unguaranteed global patching of a shared table instead.
//!
//! Only tables with pointer-sized entries are supported; Clang relative vtables use
//! displacements that are invalid once copied elsewhere.

use alloc::alloc::{alloc, dealloc, handle_alloc_error};
use core::alloc::Layout;
use core::ffi::c_void;
use core::mem::size_of;
use core::ptr::NonNull;

const ENTRY: usize = size_of::<*const c_void>();

/// A heap copy of a vtable prefix and its function entries.
///
/// The copy owns its storage. Objects pointing at it are not tracked: callers of
/// [`swap_vtable`] must restore the original table before dropping the copy.
pub struct ShadowVtable {
    allocation: NonNull<u8>,
    layout: Layout,
    prefix_size: usize,
    entries: usize,
}

// SAFETY: The copy holds only addresses; mutation requires `&mut self`.
unsafe impl Send for ShadowVtable {}
// SAFETY: Shared accesses only read the table storage.
unsafe impl Sync for ShadowVtable {}

impl ShadowVtable {
    /// Allocate uninitialized storage; `None` for an unsupported shape.
    pub(crate) fn allocate(prefix_size: usize, entries: usize) -> Option<Self> {
        if entries == 0 || prefix_size % ENTRY != 0 {
            return None;
        }
        let size = entries.checked_mul(ENTRY)?.checked_add(prefix_size)?;
        let layout = Layout::from_size_align(size, ENTRY).ok()?;
        // SAFETY: The layout has a nonzero size.
        let raw = unsafe { alloc(layout) };
        let allocation = NonNull::new(raw).unwrap_or_else(|| handle_alloc_error(layout));
        Some(Self {
            allocation,
            layout,
            prefix_size,
            entries,
        })
    }

    /// Copy a native pointer-entry vtable.
    ///
    /// Copies `prefix_size` bytes before `address_point` and `entries` function
    /// entries from it. The RTTI prefix size is
    /// [`crate::rtti::RttiMetadata::prefix_size`]; tables of classes with Itanium
    /// virtual bases need the additional offset entries preceding it.
    ///
    /// # Safety
    ///
    /// Every copied byte must be readable. Entries must be pointer-sized function
    /// addresses rather than relative displacements or authenticated pointers.
    ///
    /// # Panics
    ///
    /// Panics if `entries` is zero or `prefix_size` is not a multiple of the pointer size.
    #[must_use]
    pub unsafe fn copy_native(
        address_point: *const c_void,
        prefix_size: usize,
        entries: usize,
    ) -> Self {
        let table = Self::allocate(prefix_size, entries)
            .expect("a vtable copy needs entries and a pointer-aligned prefix");
        // SAFETY: The caller makes the source readable; the new allocation is disjoint
        // and has exactly this size.
        unsafe {
            core::ptr::copy_nonoverlapping(
                address_point.cast::<u8>().sub(prefix_size),
                table.allocation.as_ptr(),
                table.layout.size(),
            );
        }
        table
    }

    /// Start of the prefix bytes.
    pub(crate) fn prefix(&self) -> *mut u8 {
        self.allocation.as_ptr()
    }

    /// The function address point to store in an object's vtable pointer.
    #[must_use]
    pub fn address_point(&self) -> *const c_void {
        self.entries_ptr().cast_const().cast()
    }

    /// Number of function entries.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.entries
    }

    /// Read a function entry.
    ///
    /// # Panics
    ///
    /// Panics if `slot` is out of range.
    #[must_use]
    pub fn entry(&self, slot: usize) -> *const c_void {
        assert!(slot < self.entries, "vtable slot out of range");
        // SAFETY: The slot is inside the initialized, pointer-aligned entries.
        unsafe { self.entries_ptr().add(slot).read() }
    }

    /// Replace a function entry and return the previous one.
    ///
    /// The previous entry is typically the original method a hook forwards to.
    /// Calling through the table remains unsafe; the caller of [`swap_vtable`] vouches
    /// for matching signatures.
    ///
    /// # Safety
    ///
    /// No thread may call through or read this entry concurrently, including native
    /// callers of objects the table is installed on.
    ///
    /// # Panics
    ///
    /// Panics if `slot` is out of range.
    pub unsafe fn replace(&mut self, slot: usize, entry: *const c_void) -> *const c_void {
        assert!(slot < self.entries, "vtable slot out of range");
        // SAFETY: The slot is inside the pointer-aligned entries, and the caller
        // excludes concurrent readers.
        unsafe { self.entries_ptr().add(slot).replace(entry) }
    }

    fn entries_ptr(&self) -> *mut *const c_void {
        // SAFETY: The prefix is followed by the entries in this allocation.
        unsafe { self.allocation.as_ptr().add(self.prefix_size).cast() }
    }
}

impl Drop for ShadowVtable {
    fn drop(&mut self) {
        // SAFETY: This value uniquely owns the allocation with its original layout.
        unsafe { dealloc(self.allocation.as_ptr(), self.layout) };
    }
}

/// Store a new vtable address point in an object and return the previous one.
///
/// Restore the returned address point with another call before the replacement
/// table is dropped or modified.
///
/// # Safety
///
/// `object` must be a live polymorphic interface whose vtable pointer is its first
/// field, with no concurrent access to that field. `address_point` must remain valid
/// while installed and provide every entry and prefix field that any caller of the
/// object may use, with matching signatures and calling conventions.
#[must_use = "restore the previous address point before dropping the replacement"]
pub unsafe fn swap_vtable(object: *mut c_void, address_point: *const c_void) -> *const c_void {
    // SAFETY: The caller guarantees exclusive access to the object's vtable pointer.
    unsafe { object.cast::<*const c_void>().replace(address_point) }
}

/// Overwrite one entry of a shared vtable in place and return the previous entry.
///
/// This affects every object using the table. It is not guaranteed to work:
/// compiler-produced vtables normally live in read-only memory, and native callers may
/// have devirtualized or inlined the method. Making the memory writable, and
/// synchronizing with other threads calling through the table, is the caller's job.
///
/// # Safety
///
/// `address_point` must be a pointer-entry vtable with at least `slot + 1` entries,
/// writable for the duration of the call, with no concurrent access to that entry.
/// `entry` must have the replaced method's signature and calling convention.
#[must_use = "the previous entry is needed to forward to or restore the original"]
pub unsafe fn patch_vtable_entry(
    address_point: *const c_void,
    slot: usize,
    entry: *const c_void,
) -> *const c_void {
    // SAFETY: The caller guarantees a writable, exclusively accessed entry.
    unsafe {
        address_point
            .cast_mut()
            .cast::<*const c_void>()
            .add(slot)
            .replace(entry)
    }
}
