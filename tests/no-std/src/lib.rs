//! Compile and run generated pointer, inline, and COM interfaces in a no_std consumer.

#![no_std]

use cppvtable::{OwnedObject, implement, interface};
use cppvtable_com::{ComObject, IUnknown, RefCounted, SingleRefCount};

/// A pointer-to-table C interface.
#[interface(abi = c)]
pub unsafe trait IPointer {
    /// Read the stored value.
    fn value(&self) -> u32;
}

/// A C interface with an embedded callback table.
#[interface(abi = c, layout = inline)]
pub unsafe trait IInline {
    /// Read the stored value plus one.
    fn next(&self) -> u32;
}

/// A standalone ABI declaration for the inline callback header.
#[cppvtable_abi::interface(abi = c, layout = inline)]
pub unsafe trait IAbiInline {
    /// Read the stored value plus one.
    fn next(&self) -> u32;
}

#[implement(IPointer, IInline)]
struct Value {
    value: u32,
}

impl IPointerImpl for Value {
    fn value(&self) -> u32 {
        self.value
    }
}

impl IInlineImpl for Value {
    fn next(&self) -> u32 {
        self.value + 1
    }
}

/// A native C++ interface used to type-check RTTI-enabled allocation on bare metal.
#[interface(abi = cpp)]
pub unsafe trait ICppValue {
    /// Read the stored value.
    fn cpp_value(&self) -> u32;
}

/// Rust implementation whose native C++ metadata is supplied by the application.
#[implement(ICppValue)]
pub struct CppValue {
    value: u32,
}

impl ICppValueImpl for CppValue {
    fn cpp_value(&self) -> u32 {
        self.value
    }
}

/// Compile the native RTTI class constructor without inventing metadata or calling it.
///
/// # Safety
///
/// `metadata` must satisfy [`cppvtable::rtti::RttiClass::new`] for this single C++
/// interface, including the inheritance, offset, and callback contracts.
pub unsafe fn native_class(
    metadata: cppvtable::rtti::RttiMetadata,
) -> Result<cppvtable::rtti::RttiClass<CppValue>, cppvtable::rtti::RttiError> {
    // SAFETY: The caller supplies matching native metadata under the class contract.
    unsafe { cppvtable::rtti::RttiClass::new(&[Some(metadata)]) }
}

/// Allocate an object that shares the tables of `class`.
#[must_use]
pub fn with_native_rtti(
    value: u32,
    class: &cppvtable::rtti::RttiClass<CppValue>,
) -> cppvtable::rtti::RttiObject<'_, CppValue> {
    cppvtable::rtti::RttiObject::new(CppValue { value }, class)
}

/// Hook one object's C++ interface through a copy of its table.
///
/// Returns the copy, which must stay alive until the original table is restored.
///
/// # Safety
///
/// `object` must be a live native interface with `entries` pointer entries and an
/// RTTI prefix of `prefix_size` bytes; `hook` must match slot zero's signature.
pub unsafe fn hook_first_entry(
    object: *mut core::ffi::c_void,
    prefix_size: usize,
    entries: usize,
    hook: *const core::ffi::c_void,
) -> (cppvtable::hook::ShadowVtable, *const core::ffi::c_void) {
    // SAFETY: The caller guarantees the table shape, signature, and exclusive access.
    unsafe {
        let table = object.cast::<*const core::ffi::c_void>().read();
        let mut shadow = cppvtable::hook::ShadowVtable::copy_native(table, prefix_size, entries);
        let _ = shadow.replace(0, hook);
        let previous = cppvtable::hook::swap_vtable(object, shadow.address_point());
        (shadow, previous)
    }
}

/// A COM interface using the crate's platform-specific GUID and HRESULT definitions.
#[cppvtable_com::interface(abi = com, iid = "e0200001-0000-4000-8000-000000000001")]
pub unsafe trait IComValue {
    /// Read the stored value.
    fn Value(&self) -> u32;
}

/// A second COM interface that shares the same allocation.
#[cppvtable_com::interface(abi = com, iid = "e0200002-0000-4000-8000-000000000002")]
pub unsafe trait IComNext {
    /// Read the stored value plus one.
    fn Next(&self) -> u32;
}

#[cppvtable_com::implement(IComValue, IComNext)]
struct ComValue {
    value: u32,
}

// SAFETY: Default standalone hooks return no auxiliary pointers or thread contract.
unsafe impl RefCounted for ComValue {
    type Policy = SingleRefCount;
}

impl IComValueImpl for ComValue {
    fn Value(&self) -> u32 {
        self.value
    }
}

impl IComNextImpl for ComValue {
    fn Next(&self) -> u32 {
        self.value + 1
    }
}

/// Exercise allocation, both C header layouts, ABI-only calls, and COM ownership.
///
/// A final executable must supply the allocator used by the two object libraries.
#[must_use]
pub fn exercise_objects() -> u32 {
    let owner = OwnedObject::new(Value { value: 17 });
    let pointer = owner.interface::<IPointer>();
    let inline = owner.interface::<IInline>();
    let raw_inline = owner.as_raw::<IInline>();
    // SAFETY: The ABI declaration matches the live owner's immutable inline header.
    let abi_inline = unsafe { IAbiInline::from_raw_ref(&raw_inline) };

    let com = ComObject::new(ComValue { value: 19 });
    let copy = com.clone();
    let next = com.cast::<IComNext>().unwrap();
    let identity = next.cast::<IUnknown>().unwrap();
    assert_eq!(identity.as_raw(), com.as_raw());
    // SAFETY: All owning objects remain live, all declared methods have no arguments,
    // and the borrowed ABI header matches its inline storage layout.
    let sum =
        unsafe { pointer.value() + inline.next() + abi_inline.next() + copy.Value() + next.Next() };
    drop((identity, next, copy, com, owner));
    sum
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_interfaces_work_in_a_no_std_library() {
        assert_eq!(super::exercise_objects(), 92);
    }
}
