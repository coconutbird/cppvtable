//! Native C++ RTTI witnesses and runtime adapters.

use cpp::cpp;
use std::ffi::c_void;

#[cfg(test)]
mod foreign;
#[cfg(test)]
mod smoke;

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
    #ifndef _MSC_VER
    #include <cxxabi.h>
    #else
    extern "C" void* __cdecl __RTDynamicCast(void*, long, void*, void*, int) noexcept(false);
    #endif

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

    #ifdef _MSC_VER
    extern "C" void* cppvtable_rtti_runtime_msvc(void* object, std::int32_t delta,
        const void* source, const void* target, std::int32_t reference) {
        return __RTDynamicCast(object, delta, const_cast<void*>(source), const_cast<void*>(target), reference);
    }
    #else
    extern "C" void* cppvtable_rtti_runtime_itanium(const void* object, const void* source,
        const void* target, std::ptrdiff_t hint) {
        return __cxxabiv1::__dynamic_cast(object,
            static_cast<const __cxxabiv1::__class_type_info*>(source),
            static_cast<const __cxxabiv1::__class_type_info*>(target), hint);
    }
    #endif
}}

fn create_native(kind: u32) -> NativeObject {
    cpp!(unsafe [kind as "std::uint32_t"] -> NativeObject as "CppvtableRttiPointers" {
        switch (kind) {
        case 0: {
            auto* object = new CppvtableRttiWitness;
            return {object, static_cast<CppvtableRttiDerived*>(object), static_cast<CppvtableRttiSecondary*>(object)};
        }
        case 1: {
            auto* object = new CppvtableRttiOtherWitness;
            return {object, static_cast<CppvtableRttiDerived*>(object), static_cast<CppvtableRttiSecondary*>(object)};
        }
        case 2: {
            auto* object = new CppvtableRttiVirtualWitness;
            return {object, static_cast<CppvtableRttiRoot*>(object), static_cast<CppvtableRttiSecondary*>(object)};
        }
        case 3: {
            auto* object = new CppvtableRttiAmbiguousWitness;
            return {object, static_cast<CppvtableRttiRoot*>(static_cast<CppvtableRttiLeft*>(object)), static_cast<CppvtableRttiSecondary*>(object)};
        }
        case 4: {
            auto* object = new CppvtableRttiPrivateWitness;
            return {object, object->private_root(), static_cast<CppvtableRttiSecondary*>(object)};
        }
        case 5: {
            auto* object = new CppvtableRttiLeaf;
            return {object, object, nullptr};
        }
        case 6: {
            auto* object = new CppvtableRttiSingle;
            return {object, static_cast<CppvtableRttiLeaf*>(object), nullptr};
        }
        default: return {nullptr, nullptr, nullptr};
        }
    })
}

/// # Safety
/// `object` must be the uniquely owned complete allocation from the factory for `kind`.
unsafe fn delete_native(object: *mut c_void, kind: u32) {
    cpp!(unsafe [object as "void*", kind as "std::uint32_t"] {
        switch (kind) {
        case 0: CppvtableRttiDelete(static_cast<CppvtableRttiWitness*>(object)); break;
        case 1: CppvtableRttiDelete(static_cast<CppvtableRttiOtherWitness*>(object)); break;
        case 2: CppvtableRttiDelete(static_cast<CppvtableRttiVirtualWitness*>(object)); break;
        case 3: CppvtableRttiDelete(static_cast<CppvtableRttiAmbiguousWitness*>(object)); break;
        case 4: CppvtableRttiDelete(static_cast<CppvtableRttiPrivateWitness*>(object)); break;
        case 5: CppvtableRttiDelete(static_cast<CppvtableRttiLeaf*>(object)); break;
        case 6: CppvtableRttiDelete(static_cast<CppvtableRttiSingle*>(object)); break;
        }
    });
}

fn type_descriptor(kind: u32) -> *const c_void {
    cpp!(unsafe [kind as "std::uint32_t"] -> *const c_void as "const void*" {
        switch (kind) {
        case 0: return &typeid(CppvtableRttiRoot);
        case 1: return &typeid(CppvtableRttiDerived);
        case 2: return &typeid(CppvtableRttiSecondary);
        case 3: return &typeid(CppvtableRttiWitness);
        case 4: return &typeid(CppvtableRttiUnrelated);
        case 5: return &typeid(CppvtableRttiPrivateWitness);
        case 6: return &typeid(CppvtableRttiVirtualWitness);
        case 7: return &typeid(CppvtableRttiLeaf);
        case 8: return &typeid(CppvtableRttiSingle);
        case 9: return &typeid(CppvtableRttiAmbiguousWitness);
        default: return nullptr;
        }
    })
}
