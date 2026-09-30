//! Native C++ RTTI witnesses and runtime adapters.

use cpp::cpp;
use std::ffi::c_void;

#[cfg(test)]
mod cast;
#[cfg(test)]
mod foreign;
#[cfg(test)]
mod hook;
#[cfg(all(test, has_relative_vtables))]
mod relative;
#[cfg(test)]
mod smoke;

/// C++ fixture classes; the discriminants select the matching C++ `switch` cases.
#[derive(Clone, Copy, Debug)]
#[repr(u32)]
enum Class {
    Root = 0,
    Derived = 1,
    Secondary = 2,
    Unrelated = 3,
    Witness = 4,
    OtherWitness = 5,
    VirtualWitness = 6,
    AmbiguousWitness = 7,
    PrivateWitness = 8,
    Leaf = 9,
    Single = 10,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NativeObject {
    complete: *mut c_void,
    root: *mut c_void,
    secondary: *mut c_void,
}

cpp! {{
    #include <cstddef>
    #include <cstdint>
    #include <new>
    #include <typeinfo>

    class CppvtableRttiRoot {
    public:
        virtual int root_value() = 0;
    };
    class CppvtableRttiDerived : public CppvtableRttiRoot {
    public:
        virtual int derived_value() = 0;
    };
    class CppvtableRttiSecondary {
    public:
        virtual int secondary_value() = 0;
    };
    class CppvtableRttiUnrelated {
    public:
        virtual int unrelated_value() = 0;
    };
    // Nonfinal witnesses retain virtual dispatch through the declared interfaces.
    class CppvtableRttiWitness : public CppvtableRttiDerived, public CppvtableRttiSecondary {
    public:
        int root_value() override { return 11; }
        int derived_value() override { return 22; }
        int secondary_value() override { return 33; }
    };
    class CppvtableRttiOtherWitness final : public CppvtableRttiDerived, public CppvtableRttiSecondary {
    public:
        int root_value() override { return 14; }
        int derived_value() override { return 25; }
        int secondary_value() override { return 36; }
    };
    class CppvtableRttiVirtualBranch : public virtual CppvtableRttiRoot {
    public:
        virtual int branch_value() { return 44; }
    };
    class CppvtableRttiVirtualWitness final : public CppvtableRttiVirtualBranch, public CppvtableRttiSecondary {
    public:
        int root_value() override { return 11; }
        int secondary_value() override { return 33; }
    };
    class CppvtableRttiLeft : public CppvtableRttiRoot {};
    class CppvtableRttiRight : public CppvtableRttiRoot {};
    class CppvtableRttiAmbiguousWitness final : public CppvtableRttiLeft, public CppvtableRttiRight, public CppvtableRttiSecondary {
    public:
        int root_value() override { return 11; }
        int secondary_value() override { return 33; }
    };
    class CppvtableRttiPrivateWitness final : private CppvtableRttiRoot, public CppvtableRttiSecondary {
    public:
        int root_value() override { return 11; }
        int secondary_value() override { return 33; }
        CppvtableRttiRoot* private_root() { return this; }
    };
    class CppvtableRttiLeaf {
    public:
        virtual int leaf_value() { return 31; }
    };
    class CppvtableRttiSingle : public CppvtableRttiLeaf {
    public:
        virtual int single_value() { return 32; }
    };
    struct CppvtableRttiPointers { void* complete; void* root; void* secondary; };
    template<class T> void CppvtableRttiDelete(T* object) {
        // The factory retains the exact concrete type; no virtual destructor is needed.
        object->T::~T();
        ::operator delete(object);
    }
}}

/// Construct a concrete fixture class; release it with [`delete_native`].
fn create_native(class: Class) -> NativeObject {
    let kind = class as u32;
    let native = cpp!(unsafe [kind as "std::uint32_t"] -> NativeObject as "CppvtableRttiPointers" {
        switch (kind) {
        case 4: {
            auto* object = new CppvtableRttiWitness;
            return {object, static_cast<CppvtableRttiDerived*>(object), static_cast<CppvtableRttiSecondary*>(object)};
        }
        case 5: {
            auto* object = new CppvtableRttiOtherWitness;
            return {object, static_cast<CppvtableRttiDerived*>(object), static_cast<CppvtableRttiSecondary*>(object)};
        }
        case 6: {
            auto* object = new CppvtableRttiVirtualWitness;
            return {object, static_cast<CppvtableRttiRoot*>(object), static_cast<CppvtableRttiSecondary*>(object)};
        }
        case 7: {
            auto* object = new CppvtableRttiAmbiguousWitness;
            return {object, static_cast<CppvtableRttiRoot*>(static_cast<CppvtableRttiLeft*>(object)), static_cast<CppvtableRttiSecondary*>(object)};
        }
        case 8: {
            auto* object = new CppvtableRttiPrivateWitness;
            return {object, object->private_root(), static_cast<CppvtableRttiSecondary*>(object)};
        }
        case 9: {
            auto* object = new CppvtableRttiLeaf;
            return {object, object, nullptr};
        }
        case 10: {
            auto* object = new CppvtableRttiSingle;
            return {object, static_cast<CppvtableRttiLeaf*>(object), nullptr};
        }
        default: return {nullptr, nullptr, nullptr};
        }
    });
    assert!(!native.complete.is_null(), "{class:?} is abstract");
    native
}

/// # Safety
/// `object` must be the uniquely owned complete allocation from the factory for `class`.
unsafe fn delete_native(object: *mut c_void, class: Class) {
    let kind = class as u32;
    cpp!(unsafe [object as "void*", kind as "std::uint32_t"] {
        switch (kind) {
        case 4: CppvtableRttiDelete(static_cast<CppvtableRttiWitness*>(object)); break;
        case 5: CppvtableRttiDelete(static_cast<CppvtableRttiOtherWitness*>(object)); break;
        case 6: CppvtableRttiDelete(static_cast<CppvtableRttiVirtualWitness*>(object)); break;
        case 7: CppvtableRttiDelete(static_cast<CppvtableRttiAmbiguousWitness*>(object)); break;
        case 8: CppvtableRttiDelete(static_cast<CppvtableRttiPrivateWitness*>(object)); break;
        case 9: CppvtableRttiDelete(static_cast<CppvtableRttiLeaf*>(object)); break;
        case 10: CppvtableRttiDelete(static_cast<CppvtableRttiSingle*>(object)); break;
        }
    });
}

/// The native `typeid` descriptor of a fixture class.
fn type_descriptor(class: Class) -> *const c_void {
    let kind = class as u32;
    cpp!(unsafe [kind as "std::uint32_t"] -> *const c_void as "const void*" {
        switch (kind) {
        case 0: return &typeid(CppvtableRttiRoot);
        case 1: return &typeid(CppvtableRttiDerived);
        case 2: return &typeid(CppvtableRttiSecondary);
        case 3: return &typeid(CppvtableRttiUnrelated);
        case 4: return &typeid(CppvtableRttiWitness);
        case 5: return &typeid(CppvtableRttiOtherWitness);
        case 6: return &typeid(CppvtableRttiVirtualWitness);
        case 7: return &typeid(CppvtableRttiAmbiguousWitness);
        case 8: return &typeid(CppvtableRttiPrivateWitness);
        case 9: return &typeid(CppvtableRttiLeaf);
        case 10: return &typeid(CppvtableRttiSingle);
        default: return nullptr;
        }
    })
}
