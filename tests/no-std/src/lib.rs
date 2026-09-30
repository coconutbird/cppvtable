//! Compile and run generated pointer, inline, and COM interfaces in a no_std consumer.

#![no_std]

use cppvtable::{OwnedObject, implement, interface, vtable_fn};
use cppvtable_com::{ComObject, IUnknown};

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

/// Compile the native RTTI class builder without inventing metadata or calling it.
///
/// # Safety
///
/// `metadata` must satisfy [`cppvtable::rtti::RttiClassBuilder::build`] for this single
/// C++ interface, including the inheritance, offset, and callback contracts.
pub unsafe fn native_class(
    metadata: cppvtable::rtti::RttiMetadata,
) -> Result<cppvtable::rtti::RttiClass<CppValue>, cppvtable::rtti::RttiError> {
    // SAFETY: The caller supplies matching native metadata under the class contract.
    unsafe {
        cppvtable::rtti::RttiClass::<CppValue>::builder()
            .with::<ICppValue>(metadata)
            .build()
    }
}

/// Allocate an object that shares the tables of `class`.
#[must_use]
pub fn with_native_rtti(
    value: u32,
    class: &cppvtable::rtti::RttiClass<CppValue>,
) -> cppvtable::rtti::RttiObject<'_, CppValue> {
    cppvtable::rtti::RttiObject::new(CppValue { value }, class)
}

/// Replacement for [`ICppValue::cpp_value`] with the target's C++ method convention.
#[vtable_fn(abi = cpp)]
unsafe fn hooked_cpp_value(_this: *mut core::ffi::c_void) -> u32 {
    0
}

/// Replace `cpp_value` in one object's C++ interface table.
///
/// Returns the hook; dropping it restores the original table.
///
/// # Safety
///
/// `object` must satisfy [`cppvtable::hook::VtableHook::new`] for `mode`, including an
/// ordinary RTTI prefix such as the one an [`cppvtable::rtti::RttiObject`] carries.
#[must_use = "dropping the hook restores the original table"]
pub unsafe fn hook_first_entry(
    object: &ICppValue,
    mode: cppvtable::hook::HookMode,
) -> cppvtable::hook::VtableHook<'_, ICppValue> {
    // SAFETY: The caller guarantees the table shape and exclusive access; the
    // replacement has the field's convention and no preconditions.
    unsafe {
        let mut hook = object.hook(mode);
        hook.set(|table| table.cpp_value = hooked_cpp_value);
        hook
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

#[cppvtable_com::implement(IComValue, IComNext, refcount = single)]
struct ComValue {
    value: u32,
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
    let abi_inline = unsafe { IAbiInline::from_raw(raw_inline) }.unwrap();

    let com = ComObject::new(ComValue { value: 19 });
    let copy = com.clone();
    let next = com.cast::<IComNext>().unwrap();
    let identity = next.cast::<IUnknown>().unwrap();
    assert_eq!(identity.as_raw(), com.as_raw());
    let sum = pointer.value() + inline.next() + abi_inline.next() + copy.Value() + next.Next();
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
