//! Installing Rust implementations of virtual methods on native C++ objects.
//!
//! A hook copies an object's current table, including the RTTI prefix before its
//! address point, and replaces selected entries until it is dropped. [`HookMode`]
//! chooses where replacements go:
//!
//! - [`HookMode::Shadow`] points one object at the copy. Other objects of the class
//!   are unaffected, and `typeid` and `dynamic_cast` keep working because the native
//!   prefix is copied unchanged.
//! - [`HookMode::Patch`] overwrites the shared table in place and keeps the copy as a
//!   backup. It affects every object using the table and is not guaranteed to work.
//!
//! Dropping a hook restores the object's table pointer (shadow) or the patched
//! entries (patch).
//!
//! [`VtableHook`] is the typed API for a declared pointer-layout interface. It sizes
//! the copy from the interface's vtable type and its C++ ABI's RTTI prefix, edits
//! entries as fields with [`VtableHook::set`], and returns the unhooked table from
//! [`VtableHook::original`] for forwarding. Native interfaces also provide
//! `iface.hook(mode)` as a shorthand for [`VtableHook::new`]. Replacement functions
//! must have the field's exact type; [`crate::vtable_fn`] writes one function with
//! the convention the interface uses on each target.
//!
//! ```
//! use core::ffi::c_void;
//! use cppvtable::hook::{HookMode, VtableHook};
//! use cppvtable::{OwnedObject, implement, interface};
//!
//! #[interface(abi = c)]
//! unsafe trait IValue {
//!     fn value(&self) -> u32;
//! }
//! #[implement(IValue)]
//! struct Value;
//! impl IValueImpl for Value {
//!     fn value(&self) -> u32 { 1 }
//! }
//!
//! unsafe extern "C" fn replaced(_this: *mut c_void) -> u32 { 2 }
//!
//! let owner = OwnedObject::new(Value);
//! let value = owner.interface::<IValue>();
//! // SAFETY: The owner keeps the object alive until the hook drops, and nothing else
//! // reads its table pointer meanwhile. C tables have no RTTI prefix.
//! let mut hook = unsafe { VtableHook::new(&*value, HookMode::Shadow) };
//! // SAFETY: `replaced` has no preconditions, matching the safe `value` method, and no
//! // call or table borrow is concurrent with the edit.
//! unsafe { hook.set(|table| table.value = replaced) };
//! assert_eq!(value.value(), 2);
//! drop(hook);
//! assert_eq!(value.value(), 1);
//! ```
//!
//! [`RawVtableHook`] is the untyped form: it takes a raw object pointer, an explicit
//! prefix size and entry count, and installs entries by index as addresses.
//!
//! Only tables with pointer-sized entries are supported; Clang relative vtables use
//! displacements that are invalid once copied elsewhere.

use alloc::alloc::{alloc, dealloc, handle_alloc_error};
use core::alloc::Layout;
use core::ffi::c_void;
use core::fmt;
use core::marker::PhantomData;
use core::mem::{ManuallyDrop, align_of, size_of};
use core::ptr::NonNull;

use crate::rtti::{CppAbi, ItaniumPrefix, MsvcPrefix};
use crate::{Interface, VtableLayout};

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

impl fmt::Debug for ShadowVtable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:p}", self.address_point())
    }
}

/// Where a hook installs replacement entries.
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

/// Replaced virtual methods of one native object, addressed by entry index and
/// restored on drop.
///
/// This is the untyped form of [`VtableHook`]. Construction copies the object's
/// current table into a heap shadow. In [`HookMode::Shadow`] the object's vtable
/// pointer is repointed at the shadow, and replacements go into the shadow. In
/// [`HookMode::Patch`] the shadow is never installed: it is the untouched backup of
/// the native table, and replacements go into the native table.
///
/// Hooks on the same object or table copy each other's tables, so they must be
/// dropped in reverse installation order.
pub struct RawVtableHook {
    object: *mut c_void,
    native: *const c_void,
    mode: HookMode,
    shadow: ShadowVtable,
}

impl RawVtableHook {
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
    ///   callers, for every [`Self::hook`] and [`Self::unhook`] and for the drop.
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
        // SAFETY: The slot is in range of a readable table: the native table stays
        // valid while hooked, and the backup is owned.
        unsafe { self.original_table().add(slot).read() }
    }

    /// Install `entry` at `slot` of the active table and return the previous entry.
    ///
    /// # Safety
    ///
    /// `entry` must have the replaced method's signature and calling convention, and
    /// must honor the method's declared contract for every call a caller may make. No
    /// thread may call through or read this entry concurrently. In
    /// [`HookMode::Patch`] the native entry must be writable.
    ///
    /// # Panics
    ///
    /// Panics if `slot` is out of range.
    pub unsafe fn hook(&mut self, slot: usize, entry: *const c_void) -> *const c_void {
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
    pub unsafe fn unhook(&mut self, slot: usize) {
        let original = self.original(slot);
        // SAFETY: `original` checked the slot; the caller makes it writable and exclusive.
        unsafe { self.active().add(slot).write(original) };
    }

    fn check(&self, slot: usize) {
        assert!(slot < self.shadow.entries, "vtable slot out of range");
    }

    /// The unhooked entries: the native table, which shadow mode never writes, in
    /// [`HookMode::Shadow`], and the backup captured at construction in
    /// [`HookMode::Patch`].
    fn original_table(&self) -> *const *const c_void {
        match self.mode {
            HookMode::Shadow => self.native.cast(),
            HookMode::Patch => self.shadow.entries_ptr().cast_const(),
        }
    }

    fn active(&self) -> *mut *const c_void {
        match self.mode {
            HookMode::Shadow => self.shadow.entries_ptr(),
            HookMode::Patch => self.native.cast_mut().cast(),
        }
    }
}

impl Drop for RawVtableHook {
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

impl fmt::Debug for RawVtableHook {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RawVtableHook")
            .field("object", &self.object)
            .field("native", &self.native)
            .field("shadow", &self.shadow)
            .field("mode", &self.mode)
            .field("entries", &self.shadow.entries)
            .finish()
    }
}

/// Size of the ordinary pointer-representation RTTI prefix of `abi`, or zero for C
/// and COM tables.
const fn rtti_prefix_size(abi: Option<CppAbi>) -> usize {
    match abi {
        Some(CppAbi::Msvc) => size_of::<MsvcPrefix>(),
        Some(CppAbi::Itanium) => size_of::<ItaniumPrefix>(),
        None => 0,
    }
}

/// Replaced virtual methods of one object's interface `I`, edited as `I::Vtbl` fields
/// and restored on drop.
///
/// The hook borrows the interface, so it cannot outlive the view it was created from.
/// Entry indices used by [`Self::hook`] and [`Self::unhook`] count pointer-sized
/// entries of `I::Vtbl` from the address point, including inherited and reserved
/// entries. [`RawVtableHook`] documents how each [`HookMode`] installs entries.
pub struct VtableHook<'a, I: Interface> {
    raw: RawVtableHook,
    _object: PhantomData<&'a I>,
}

impl<'a, I: Interface> VtableHook<'a, I> {
    /// Pointer-sized entries of `I::Vtbl`, checked at compile time.
    const ENTRIES: usize = {
        assert!(
            matches!(I::LAYOUT, VtableLayout::Pointer),
            "vtable hooks require a pointer-layout interface"
        );
        assert!(
            size_of::<I::Vtbl>() % ENTRY == 0 && align_of::<I::Vtbl>() <= ENTRY,
            "vtable hooks require a table of pointer-sized entries"
        );
        size_of::<I::Vtbl>() / ENTRY
    };

    /// Hook `iface`'s object, copying the ordinary RTTI prefix of `I`'s C++ ABI.
    ///
    /// The prefix is one pointer for Microsoft tables, two words for Itanium tables,
    /// and empty for C and COM tables. Use [`Self::with_prefix`] for any other prefix,
    /// such as Itanium virtual-base offsets or a Rust object without RTTI.
    ///
    /// # Safety
    ///
    /// The contract of [`Self::with_prefix`] applies with that prefix size. A Rust
    /// [`crate::OwnedObject`] of a C++ interface has no RTTI prefix unless it was
    /// created from a [`crate::rtti::RttiClass`]; hook it with `with_prefix(.., 0)`.
    ///
    /// # Panics
    ///
    /// Panics if `I::Vtbl` has no entries.
    #[must_use = "dropping the hook restores the original table"]
    pub unsafe fn new(iface: &'a I, mode: HookMode) -> Self {
        // SAFETY: The caller upholds `with_prefix` for the ABI's ordinary prefix.
        unsafe { Self::with_prefix(iface, mode, const { rtti_prefix_size(I::CPP_ABI) }) }
    }

    /// Hook `iface`'s object, copying `prefix_size` bytes before its address point.
    ///
    /// # Safety
    ///
    /// - The object must stay live, with no concurrent access to its vtable pointer,
    ///   until the hook is dropped.
    /// - `prefix_size` bytes before the address point and all of `I::Vtbl` must be
    ///   readable, and the table must stay valid until the hook is dropped. Its
    ///   entries must be pointer-sized function addresses, not Clang relative
    ///   displacements or authenticated pointers.
    /// - In [`HookMode::Patch`] the native table must be writable, with no concurrent
    ///   callers, for every [`Self::set`], [`Self::hook`], and [`Self::unhook`] and for
    ///   the drop. The edits affect every object sharing the table.
    /// - Hooks on the same object or table must be dropped in reverse installation order.
    ///
    /// # Panics
    ///
    /// Panics if `I::Vtbl` has no entries or `prefix_size` is not a multiple of the
    /// pointer size.
    #[must_use = "dropping the hook restores the original table"]
    pub unsafe fn with_prefix(iface: &'a I, mode: HookMode, prefix_size: usize) -> Self {
        let object = cppvtable_abi::interface::raw_of(iface);
        Self {
            // SAFETY: A pointer-layout interface's first field is its vtable pointer,
            // and `I::Vtbl` spans `ENTRIES` entries; the caller upholds the rest.
            raw: unsafe { RawVtableHook::new(object, prefix_size, Self::ENTRIES, mode) },
            _object: PhantomData,
        }
    }

    /// Where replacement entries are installed.
    #[must_use]
    pub fn mode(&self) -> HookMode {
        self.raw.mode()
    }

    /// Number of hookable pointer-sized entries in `I::Vtbl`.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.raw.entry_count()
    }

    /// The unhooked table, whose entries hooks forward to.
    ///
    /// This is the native table in [`HookMode::Shadow`], which shadow mode never
    /// writes, and the backup captured at construction in [`HookMode::Patch`]. It is
    /// never the table this hook edits.
    #[must_use]
    pub fn original(&self) -> &I::Vtbl {
        // SAFETY: The table holds `ENTRIES` entries forming an aligned `I::Vtbl`:
        // the native table stays valid while hooked, and the backup is owned and
        // pointer-aligned. This hook never writes either one.
        unsafe { &*self.raw.original_table().cast::<I::Vtbl>() }
    }

    /// Edit the active table as an `I::Vtbl`.
    ///
    /// `edit` receives a copy of the table the object calls through. Afterwards only
    /// entries whose words changed are written back. If `edit` panics, nothing is
    /// written.
    ///
    /// # Safety
    ///
    /// - Every replacement must have its field's signature and calling convention and
    ///   must honor the replaced method's declared contract: callers of a method
    ///   declared as a safe `fn` may call it with any arguments, so the replacement
    ///   may not add preconditions. Replacements must not unwind.
    /// - No thread may call through or read the active table during the call, and no
    ///   borrow of it may be live, such as a view's `vtable()` or another hook's
    ///   [`Self::original`] of the same table.
    /// - In [`HookMode::Patch`] the native table must be writable.
    pub unsafe fn set(&mut self, edit: impl FnOnce(&mut I::Vtbl)) {
        let active = self.raw.active();
        // SAFETY: The active table holds `ENTRIES` valid entries forming an aligned
        // `I::Vtbl`, and the caller excludes concurrent writers. The copy is never
        // dropped, so no destructor runs on the bitwise duplicate.
        let mut table = ManuallyDrop::new(unsafe { active.cast::<I::Vtbl>().read() });
        edit(&mut *table);
        let edited = core::ptr::from_ref::<I::Vtbl>(&*table).cast::<*const c_void>();
        for slot in 0..Self::ENTRIES {
            // SAFETY: Both tables consist of `ENTRIES` pointer-sized entries with no
            // padding; the caller makes the active table writable and exclusive.
            // Only changed entries are written to avoid needless writes to shared
            // memory.
            unsafe {
                let entry = edited.add(slot).read();
                let live = active.add(slot);
                if live.read() != entry {
                    live.write(entry);
                }
            }
        }
    }

    /// Install `entry` at entry index `slot` of the active table and return the
    /// previous entry.
    ///
    /// # Safety
    ///
    /// `entry` must be a valid value of the `I::Vtbl` field at that index, with its
    /// signature and calling convention, and must honor the replaced method's
    /// declared contract as described for [`Self::set`]. No thread may call through
    /// or read this entry concurrently, and no borrow of the active table may be live.
    /// In [`HookMode::Patch`] the native entry must be writable.
    ///
    /// # Panics
    ///
    /// Panics if `slot` is out of range.
    pub unsafe fn hook(&mut self, slot: usize, entry: *const c_void) -> *const c_void {
        // SAFETY: The caller upholds the raw contract for this interface's field.
        unsafe { self.raw.hook(slot, entry) }
    }

    /// Put the original entry back at entry index `slot` of the active table.
    ///
    /// # Safety
    ///
    /// No thread may call through or read this entry concurrently, and no borrow of
    /// the active table may be live. In [`HookMode::Patch`] the native entry must be
    /// writable.
    ///
    /// # Panics
    ///
    /// Panics if `slot` is out of range.
    pub unsafe fn unhook(&mut self, slot: usize) {
        // SAFETY: The caller upholds the raw contract.
        unsafe { self.raw.unhook(slot) };
    }
}

impl<I: Interface> fmt::Debug for VtableHook<'_, I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VtableHook")
            .field("interface", &I::NAME)
            .field("raw", &self.raw)
            .finish()
    }
}
