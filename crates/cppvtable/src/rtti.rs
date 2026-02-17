//! Runtime Type Information (RTTI) for C++ vtable interop
//!
//! This module provides RTTI support matching the MSVC ABI layout.
//! TypeInfo is stored at slot -1 (negative offset from vtable pointer).
//!
//! ## Memory Layout
//!
//! ```text
//! VTable in memory:
//! ┌─────────────────┐
//! │ TypeInfo*       │  ← slot -1 (offset -8 on x64, -4 on x86)
//! ├─────────────────┤
//! │ method_0        │  ← slot 0 (vtable pointer points here)
//! │ method_1        │  ← slot 1
//! │ ...             │
//! └─────────────────┘
//! ```
//!
//! The object's vtable pointer points to slot 0. To access TypeInfo,
//! we read the pointer at offset -1.

use std::ffi::c_void;

/// Information about a single interface implementation
#[repr(C)]
#[derive(Clone, Copy)]
pub struct InterfaceInfo {
    /// Unique identifier for the interface (address of a static marker)
    pub interface_id: *const u8,
    /// Byte offset from object start to this interface's vtable pointer
    pub offset: isize,
}

// SAFETY: InterfaceInfo only contains a pointer to a static and an offset
unsafe impl Send for InterfaceInfo {}
unsafe impl Sync for InterfaceInfo {}

impl std::fmt::Debug for InterfaceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InterfaceInfo")
            .field("interface_id", &(self.interface_id as usize))
            .field("offset", &self.offset)
            .finish()
    }
}

impl InterfaceInfo {
    /// Create a new InterfaceInfo
    pub const fn new(interface_id: *const u8, offset: isize) -> Self {
        Self {
            interface_id,
            offset,
        }
    }
}

/// Runtime type information for a concrete class
#[repr(C)]
#[derive(Debug)]
pub struct TypeInfo {
    /// Unique identifier for this concrete type
    pub type_id: usize,
    /// Human-readable type name (for debugging)
    pub type_name: &'static str,
    /// List of implemented interfaces with their offsets
    pub interfaces: &'static [InterfaceInfo],
}

impl TypeInfo {
    /// Create a new TypeInfo
    pub const fn new(
        type_id: usize,
        type_name: &'static str,
        interfaces: &'static [InterfaceInfo],
    ) -> Self {
        Self {
            type_id,
            type_name,
            interfaces,
        }
    }

    /// Cast object pointer to a different interface, returns adjusted pointer or null
    ///
    /// # Safety
    /// - `object_ptr` must point to a valid instance of the type this TypeInfo describes
    pub unsafe fn cast_to(
        &self,
        object_ptr: *const c_void,
        interface_id: *const u8,
    ) -> *const c_void {
        for info in self.interfaces {
            if std::ptr::eq(info.interface_id, interface_id) {
                // SAFETY: Caller guarantees object_ptr is valid and offset is correct for this type
                return unsafe { (object_ptr as *const u8).offset(info.offset) as *const c_void };
            }
        }
        std::ptr::null()
    }

    /// Check if this type implements a given interface
    pub fn implements(&self, interface_id: *const u8) -> bool {
        self.interfaces
            .iter()
            .any(|i| std::ptr::eq(i.interface_id, interface_id))
    }
}

/// Trait for types that have RTTI
pub trait HasTypeInfo {
    /// Get the TypeInfo for this type
    fn type_info() -> &'static TypeInfo;
}

/// Retrieve TypeInfo from a vtable pointer (slot -1)
///
/// # Safety
/// - `vtable_ptr` must point to a valid vtable with TypeInfo at slot -1
/// - The vtable must have been generated with RTTI enabled
#[inline]
pub unsafe fn get_type_info(vtable_ptr: *const c_void) -> &'static TypeInfo {
    // SAFETY: Caller guarantees vtable has RTTI at slot -1
    unsafe {
        let rtti_ptr = (vtable_ptr as *const *const TypeInfo).offset(-1);
        &**rtti_ptr
    }
}

/// Helper to generate a unique interface ID from a static address
///
/// Usage: `static IID_IFOO: InterfaceId = interface_id!();`
#[macro_export]
macro_rules! interface_id {
    () => {{
        static __ID: u8 = 0;
        &__ID as *const u8 as usize
    }};
}

/// Wrapper for vtables with RTTI at slot -1
///
/// This struct is laid out so that `methods` is at offset sizeof(pointer),
/// allowing the vtable pointer to point to `methods` while `rtti` is at
/// the negative offset.
#[repr(C)]
pub struct VTableWithRtti<T> {
    /// TypeInfo pointer (slot -1 when viewed from methods pointer)
    pub rtti: *const TypeInfo,
    /// The actual vtable methods
    pub methods: T,
}

impl<T> VTableWithRtti<T> {
    /// Create a new vtable wrapper with RTTI
    pub const fn new(rtti: &'static TypeInfo, methods: T) -> Self {
        Self { rtti, methods }
    }

    /// Get a pointer to the methods (what the object's vtable pointer should store)
    pub const fn vtable_ptr(&self) -> *const T {
        &self.methods
    }
}
