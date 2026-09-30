//! Parity of the target C++ runtime's pointer `dynamic_cast` with the compiler's own.
//!
//! `DynamicCastRuntime::TARGET` must answer every downcast and cross-cast exactly as
//! C++ `dynamic_cast` does, including failures through private, ambiguous, and virtual
//! bases. Upcasts are excluded: the compiler resolves them statically, and the runtime
//! cast does not implement them.

use super::*;
use cppvtable::rtti::DynamicCastRuntime;

cpp! {{
    template<class Source> void* CppvtableRttiCastTo(Source* source, std::uint32_t target) {
        switch (target) {
        case 0: return dynamic_cast<CppvtableRttiRoot*>(source);
        case 1: return dynamic_cast<CppvtableRttiDerived*>(source);
        case 2: return dynamic_cast<CppvtableRttiSecondary*>(source);
        case 3: return dynamic_cast<CppvtableRttiUnrelated*>(source);
        case 4: return dynamic_cast<CppvtableRttiWitness*>(source);
        case 5: return dynamic_cast<CppvtableRttiOtherWitness*>(source);
        case 6: return dynamic_cast<CppvtableRttiVirtualWitness*>(source);
        case 7: return dynamic_cast<CppvtableRttiAmbiguousWitness*>(source);
        case 8: return dynamic_cast<CppvtableRttiPrivateWitness*>(source);
        case 9: return dynamic_cast<CppvtableRttiLeaf*>(source);
        case 10: return dynamic_cast<CppvtableRttiSingle*>(source);
        default: return nullptr;
        }
    }
}}

/// Every class type a cast can name.
const TARGETS: [Class; 11] = [
    Class::Root,
    Class::Derived,
    Class::Secondary,
    Class::Unrelated,
    Class::Witness,
    Class::OtherWitness,
    Class::VirtualWitness,
    Class::AmbiguousWitness,
    Class::PrivateWitness,
    Class::Leaf,
    Class::Single,
];

/// The compiler's `dynamic_cast<Target*>(static_cast<Source*>(object))`.
///
/// # Safety
/// `object` must be the live `source` subobject of a polymorphic object.
unsafe fn language_cast(object: *mut c_void, source: Class, target: Class) -> *mut c_void {
    let (source, target) = (source as u32, target as u32);
    cpp!(unsafe [object as "void*", source as "std::uint32_t", target as "std::uint32_t"] -> *mut c_void as "void*" {
        switch (source) {
        case 0: return CppvtableRttiCastTo(static_cast<CppvtableRttiRoot*>(object), target);
        case 2: return CppvtableRttiCastTo(static_cast<CppvtableRttiSecondary*>(object), target);
        case 9: return CppvtableRttiCastTo(static_cast<CppvtableRttiLeaf*>(object), target);
        default: return nullptr;
        }
    })
}

/// Assert that the runtime cast agrees with the compiler for every downcast and
/// cross-cast from one subobject, and return how many of them succeeded.
///
/// # Safety
/// `object` must be the live `source` subobject of an object with native RTTI.
pub(super) unsafe fn assert_runtime_matches_language(
    object: *mut c_void,
    source: Class,
    label: &str,
) -> usize {
    let mut succeeded = 0;
    for target in TARGETS {
        if source as u32 == target as u32 {
            continue;
        }
        // SAFETY: The caller supplies a live subobject of `source` with native RTTI,
        // and the descriptors are the compiler's own for the same runtime.
        let (expected, actual) = unsafe {
            (
                language_cast(object, source, target),
                DynamicCastRuntime::TARGET.cast(
                    object,
                    type_descriptor(source),
                    type_descriptor(target),
                ),
            )
        };
        assert_eq!(actual, expected, "{label}: {source:?} -> {target:?}");
        succeeded += usize::from(!actual.is_null());
    }
    succeeded
}

#[test]
fn the_target_runtime_casts_native_objects_like_the_compiler() {
    let mut succeeded = 0;
    for class in [
        Class::Witness,
        Class::OtherWitness,
        Class::VirtualWitness,
        Class::AmbiguousWitness,
        Class::PrivateWitness,
        Class::Leaf,
        Class::Single,
    ] {
        let native = create_native(class);
        let root = if matches!(class, Class::Leaf | Class::Single) {
            Class::Leaf
        } else {
            Class::Root
        };
        // SAFETY: The factory returns live subobjects of their declared static types,
        // and the object is deleted only after the casts.
        unsafe {
            succeeded += assert_runtime_matches_language(native.root, root, &format!("{class:?}"));
            if !native.secondary.is_null() {
                succeeded += assert_runtime_matches_language(
                    native.secondary,
                    Class::Secondary,
                    &format!("{class:?}"),
                );
            }
            delete_native(native.complete, class);
        }
    }
    assert!(succeeded > 0, "every cast failed, so parity proves nothing");
}

#[test]
fn a_null_source_casts_to_null() {
    // SAFETY: A null source is permitted and never dereferenced.
    let cast = unsafe {
        DynamicCastRuntime::TARGET.cast(
            core::ptr::null_mut(),
            type_descriptor(Class::Root),
            type_descriptor(Class::Secondary),
        )
    };
    assert!(cast.is_null());
}
