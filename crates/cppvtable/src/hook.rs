//! Installing Rust implementations of virtual methods on native C++ objects.
//!
//! A [`VtableHook`] copies an object's current table, including the RTTI prefix
//! before its address point, and replaces selected entries until it is dropped.
//! [`HookMode`] chooses where replacements go:
//!
//! - [`HookMode::Shadow`] points one object at the copy. Other objects of the class
//!   are unaffected, and `typeid` and `dynamic_cast` keep working because the native
//!   prefix is copied unchanged.
//! - [`HookMode::Patch`] overwrites the shared table in place and keeps the copy as a
//!   backup. It affects every object using the table and is not guaranteed to work.
//!
//! [`VtableHook::original`] returns the entry a hook forwards to. Dropping the hook
//! restores the object's table pointer (shadow) or the patched entries (patch).
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
/// The copy owns its storage; objects pointing at it are not tracked.
pub(crate) struct ShadowVtable {
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

    /// Copy `prefix_size` bytes before `address_point` and `entries` entries from it.
    ///
    /// # Safety
    ///
    /// Every copied byte must be readable.
    ///
    /// # Panics
    ///
    /// Panics if `entries` is zero or `prefix_size` is not a multiple of the pointer size.
    unsafe fn copy(address_point: *const c_void, prefix_size: usize, entries: usize) -> Self {
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
    pub(crate) fn address_point(&self) -> *const c_void {
        self.entries_ptr().cast_const().cast()
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

/// Where a [`VtableHook`] installs replacement entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookMode {
    /// Portable: repoint only this object at the shadow copy; other objects of the
    /// class are unaffected.
    Shadow,
    /// Overwrite entries of the shared native table in place; affects every object
    /// using it.
    ///
    /// Not guaranteed: compiler vtables normally live in read-only memory (making it
    /// writable is the caller's job) and devirtualized calls are not affected.
    Patch,
}

/// Replaced virtual methods of one native object, restored on drop.
///
/// Construction copies the object's current table into a heap shadow. In
/// [`HookMode::Shadow`] the object's vtable pointer is repointed at the shadow, and
/// replacements go into the shadow. In [`HookMode::Patch`] the shadow is never
/// installed: it is the untouched backup of the native table, and replacements go into
/// the native table.
///
/// Hooks on the same object or table copy each other's tables, so they must be
/// dropped in reverse installation order.
pub struct VtableHook {
    object: *mut c_void,
    native: *const c_void,
    mode: HookMode,
    shadow: ShadowVtable,
}

impl VtableHook {
    /// Hook `object` by copying its current table.
    ///
    /// Copies `prefix_size` bytes before the address point and `entries` pointer
    /// entries. The RTTI prefix size is [`crate::rtti::RttiMetadata::prefix_size`];
    /// tables of classes with Itanium virtual bases need the additional offset entries
    /// preceding it.
    ///
    /// # Safety
    ///
    /// - `object` must be a live polymorphic interface whose vtable pointer is its first
    ///   field. It must stay live, with no concurrent access to that field, until the
    ///   hook is dropped.
    /// - Every copied byte of its table must be readable, and the table must stay valid
    ///   until the hook is dropped. Entries must be pointer-sized function addresses,
    ///   not Clang relative displacements or authenticated pointers.
    /// - In [`HookMode::Patch`] the native entries must be writable, with no concurrent
    ///   callers, for every [`Self::replace`] and [`Self::restore`] and for the drop.
    /// - Hooks on the same object or table must be dropped in reverse installation order.
    ///
    /// # Panics
    ///
    /// Panics if `entries` is zero or `prefix_size` is not a multiple of the pointer size.
    #[must_use = "dropping the hook restores the original table"]
    pub unsafe fn new(
        object: *mut c_void,
        prefix_size: usize,
        entries: usize,
        mode: HookMode,
    ) -> Self {
        let vptr = object.cast::<*const c_void>();
        // SAFETY: The caller guarantees a live object with its vtable pointer first.
        let native = unsafe { vptr.read() };
        // SAFETY: The caller makes the prefix and entries readable.
        let shadow = unsafe { ShadowVtable::copy(native, prefix_size, entries) };
        if mode == HookMode::Shadow {
            // SAFETY: The caller excludes concurrent access to the vtable pointer; the
            // shadow outlives the installation because drop restores `native` first.
            unsafe { vptr.write(shadow.address_point()) };
        }
        Self {
            object,
            native,
            mode,
            shadow,
        }
    }

    /// Where replacement entries are installed.
    #[must_use]
    pub fn mode(&self) -> HookMode {
        self.mode
    }

    /// The table address point the object calls through while hooked: the shadow in
    /// [`HookMode::Shadow`], the native table in [`HookMode::Patch`].
    #[must_use]
    pub fn address_point(&self) -> *const c_void {
        match self.mode {
            HookMode::Shadow => self.shadow.address_point(),
            HookMode::Patch => self.native,
        }
    }

    /// Number of hookable entries.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.shadow.entries
    }

    /// The original entry at `slot`, which hooks forward to.
    ///
    /// Read from the native table, which shadow mode never writes, in
    /// [`HookMode::Shadow`], and from the backup captured at construction in
    /// [`HookMode::Patch`].
    ///
    /// # Panics
    ///
    /// Panics if `slot` is out of range.
    #[must_use]
    pub fn original(&self, slot: usize) -> *const c_void {
        self.check(slot);
        let table = match self.mode {
            HookMode::Shadow => self.native.cast::<*const c_void>(),
            HookMode::Patch => self.shadow.entries_ptr().cast_const(),
        };
        // SAFETY: The slot is in range of a readable table: the native table stays
        // valid while hooked, and the backup is owned.
        unsafe { table.add(slot).read() }
    }

    /// Install `entry` at `slot` of the active table and return the previous entry.
    ///
    /// # Safety
    ///
    /// `entry` must have the replaced method's signature and calling convention. No
    /// thread may call through or read this entry concurrently. In
    /// [`HookMode::Patch`] the native entry must be writable.
    ///
    /// # Panics
    ///
    /// Panics if `slot` is out of range.
    pub unsafe fn replace(&mut self, slot: usize, entry: *const c_void) -> *const c_void {
        self.check(slot);
        // SAFETY: The slot is in range; the caller makes it writable and exclusive.
        unsafe { self.active().add(slot).replace(entry) }
    }

    /// Put the original entry back at `slot` of the active table.
    ///
    /// # Safety
    ///
    /// No thread may call through or read this entry concurrently. In
    /// [`HookMode::Patch`] the native entry must be writable.
    ///
    /// # Panics
    ///
    /// Panics if `slot` is out of range.
    pub unsafe fn restore(&mut self, slot: usize) {
        let original = self.original(slot);
        // SAFETY: `original` checked the slot; the caller makes it writable and exclusive.
        unsafe { self.active().add(slot).write(original) };
    }

    fn check(&self, slot: usize) {
        assert!(slot < self.shadow.entries, "vtable slot out of range");
    }

    fn active(&self) -> *mut *const c_void {
        match self.mode {
            HookMode::Shadow => self.shadow.entries_ptr(),
            HookMode::Patch => self.native.cast_mut().cast(),
        }
    }
}

impl Drop for VtableHook {
    fn drop(&mut self) {
        match self.mode {
            // SAFETY: `new`'s contract keeps the object live with an exclusive vptr.
            HookMode::Shadow => unsafe { self.object.cast::<*const c_void>().write(self.native) },
            HookMode::Patch => {
                let backup = self.shadow.entries_ptr();
                let native = self.active();
                for slot in 0..self.shadow.entries {
                    // SAFETY: Both tables hold `entries` entries; `new`'s contract makes
                    // the native entries writable and exclusive during drop. Only
                    // changed slots are written to avoid needless writes to shared memory.
                    unsafe {
                        let original = backup.add(slot).read();
                        let live = native.add(slot);
                        if live.read() != original {
                            live.write(original);
                        }
                    }
                }
            }
        }
    }
}
