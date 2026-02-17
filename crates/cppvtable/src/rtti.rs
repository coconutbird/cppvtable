//! Runtime Type Information (RTTI) for C++ vtable interop
//!
//! This module provides **Rust-side RTTI** for runtime interface casting.
//! This is completely separate from C++ RTTI and does not interoperate with it.
//!
//! ## Important: No C++ RTTI Support
//!
//! This crate does **not** support C++ native RTTI (`dynamic_cast`, `typeid`).
//! C++ RTTI uses complex ABI-specific structures (MSVC's `_RTTICompleteObjectLocator`,
//! Itanium's `__class_type_info`) stored at vtable slot -1. Parsing these would require:
//! - ABI-specific code for MSVC vs GCC/Clang
//! - Walking complex class hierarchy descriptors
//! - Handling virtual inheritance offsets
//!
//! If you need to cast C++ objects at runtime, the C++ code should expose its own
//! casting mechanism (like COM's `QueryInterface`).
//!
//! ## What This Module Provides
//!
//! Rust-side type metadata for Rust objects implementing C++ interfaces:
//! - [`TypeInfo`] - describes a Rust type and its implemented interfaces
//! - [`InterfaceInfo`] - offset information for casting between interfaces
//! - [`cast_to()`](TypeInfo::cast_to) - runtime casting between interfaces
//!
//! ## Memory Layout
//!
//! ```text
//! VTable in memory (with Rust RTTI):
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

#[cfg(test)]
mod tests {
    use super::*;

    // Test interface IDs (unique static addresses)
    static IID_FIRST: u8 = 0;
    static IID_SECOND: u8 = 0;
    static IID_THIRD: u8 = 0;

    fn first_id() -> *const u8 {
        &IID_FIRST
    }
    fn second_id() -> *const u8 {
        &IID_SECOND
    }
    fn third_id() -> *const u8 {
        &IID_THIRD
    }

    #[test]
    fn test_interface_info_new() {
        let info = InterfaceInfo::new(first_id(), 8);
        assert!(std::ptr::eq(info.interface_id, first_id()));
        assert_eq!(info.offset, 8);
    }

    #[test]
    fn test_interface_info_zero_offset() {
        let info = InterfaceInfo::new(first_id(), 0);
        assert_eq!(info.offset, 0);
    }

    #[test]
    fn test_interface_info_debug() {
        let info = InterfaceInfo::new(first_id(), 16);
        let debug_str = format!("{:?}", info);
        assert!(debug_str.contains("InterfaceInfo"));
        assert!(debug_str.contains("offset"));
        assert!(debug_str.contains("16"));
    }

    #[test]
    fn test_type_info_new() {
        static INTERFACES: [InterfaceInfo; 0] = [];
        let ti = TypeInfo::new(42, "TestType", &INTERFACES);
        assert_eq!(ti.type_id, 42);
        assert_eq!(ti.type_name, "TestType");
        assert_eq!(ti.interfaces.len(), 0);
    }

    #[test]
    fn test_type_info_with_interfaces() {
        static INTERFACES: [InterfaceInfo; 2] = [
            InterfaceInfo {
                interface_id: std::ptr::null(), // Will compare by address anyway
                offset: 0,
            },
            InterfaceInfo {
                interface_id: std::ptr::null(),
                offset: 8,
            },
        ];
        let ti = TypeInfo::new(1, "MultiInterface", &INTERFACES);
        assert_eq!(ti.interfaces.len(), 2);
        assert_eq!(ti.interfaces[0].offset, 0);
        assert_eq!(ti.interfaces[1].offset, 8);
    }

    #[test]
    fn test_implements_returns_true() {
        let interfaces: &'static [InterfaceInfo] = Box::leak(Box::new([
            InterfaceInfo::new(first_id(), 0),
            InterfaceInfo::new(second_id(), 8),
        ]));
        let ti = TypeInfo::new(1, "Test", interfaces);

        assert!(ti.implements(first_id()));
        assert!(ti.implements(second_id()));
    }

    #[test]
    fn test_implements_returns_false_for_unknown() {
        let interfaces: &'static [InterfaceInfo] = Box::leak(Box::new([
            InterfaceInfo::new(first_id(), 0),
            InterfaceInfo::new(second_id(), 8),
        ]));
        let ti = TypeInfo::new(1, "Test", interfaces);

        assert!(!ti.implements(third_id()));
    }

    #[test]
    fn test_implements_empty_interfaces() {
        static INTERFACES: [InterfaceInfo; 0] = [];
        let ti = TypeInfo::new(1, "Empty", &INTERFACES);

        assert!(!ti.implements(first_id()));
    }

    #[test]
    fn test_cast_to_primary_interface() {
        let interfaces: &'static [InterfaceInfo] = Box::leak(Box::new([
            InterfaceInfo::new(first_id(), 0),
            InterfaceInfo::new(second_id(), 8),
        ]));
        let ti = TypeInfo::new(1, "Test", interfaces);

        let obj: [u8; 24] = [0; 24];
        let obj_ptr = obj.as_ptr() as *const c_void;

        unsafe {
            let result = ti.cast_to(obj_ptr, first_id());
            assert_eq!(result, obj_ptr); // Offset 0, same pointer
        }
    }

    #[test]
    fn test_cast_to_secondary_interface() {
        let interfaces: &'static [InterfaceInfo] = Box::leak(Box::new([
            InterfaceInfo::new(first_id(), 0),
            InterfaceInfo::new(second_id(), 8),
        ]));
        let ti = TypeInfo::new(1, "Test", interfaces);

        let obj: [u8; 24] = [0; 24];
        let obj_ptr = obj.as_ptr() as *const c_void;

        unsafe {
            let result = ti.cast_to(obj_ptr, second_id());
            let expected = (obj_ptr as *const u8).offset(8) as *const c_void;
            assert_eq!(result, expected);
        }
    }

    #[test]
    fn test_cast_to_unknown_returns_null() {
        let interfaces: &'static [InterfaceInfo] = Box::leak(Box::new([
            InterfaceInfo::new(first_id(), 0),
            InterfaceInfo::new(second_id(), 8),
        ]));
        let ti = TypeInfo::new(1, "Test", interfaces);

        let obj: [u8; 24] = [0; 24];
        let obj_ptr = obj.as_ptr() as *const c_void;

        unsafe {
            let result = ti.cast_to(obj_ptr, third_id());
            assert!(result.is_null());
        }
    }

    #[test]
    fn test_cast_to_empty_interfaces_returns_null() {
        static INTERFACES: [InterfaceInfo; 0] = [];
        let ti = TypeInfo::new(1, "Empty", &INTERFACES);

        let obj: [u8; 24] = [0; 24];
        let obj_ptr = obj.as_ptr() as *const c_void;

        unsafe {
            let result = ti.cast_to(obj_ptr, first_id());
            assert!(result.is_null());
        }
    }

    #[test]
    fn test_type_info_debug() {
        static INTERFACES: [InterfaceInfo; 0] = [];
        let ti = TypeInfo::new(99, "DebugTest", &INTERFACES);
        let debug_str = format!("{:?}", ti);
        assert!(debug_str.contains("TypeInfo"));
        assert!(debug_str.contains("DebugTest"));
        assert!(debug_str.contains("99"));
    }

    #[test]
    fn test_vtable_with_rtti_layout() {
        #[repr(C)]
        struct FakeVTable {
            method1: fn(),
            method2: fn(),
        }

        fn dummy() {}

        static INTERFACES: [InterfaceInfo; 0] = [];
        static TYPE_INFO: TypeInfo = TypeInfo::new(1, "Fake", &INTERFACES);

        let vtable = VTableWithRtti::new(
            &TYPE_INFO,
            FakeVTable {
                method1: dummy,
                method2: dummy,
            },
        );

        // The vtable_ptr should point to methods, not rtti
        let ptr = vtable.vtable_ptr();
        assert!(!ptr.is_null());

        // RTTI should be at negative offset from methods
        unsafe {
            let rtti_ptr = (ptr as *const *const TypeInfo).offset(-1);
            let rtti = &**rtti_ptr;
            assert_eq!(rtti.type_name, "Fake");
        }
    }

    #[test]
    fn test_interface_ids_are_unique() {
        // Each static has a unique address
        assert!(!std::ptr::eq(first_id(), second_id()));
        assert!(!std::ptr::eq(second_id(), third_id()));
        assert!(!std::ptr::eq(first_id(), third_id()));
    }
}
