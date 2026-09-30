//! Native COM vtable calls, interface identity, and lifetime in both directions.

use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use cpp::cpp;
use cppvtable_com::{RefCounted, SingleRefCount, implement, interface};

#[cfg(test)]
use cppvtable_com::{ComObject, ComPtr};

/// The primary COM interface of the compiler fixture.
#[interface(abi = com, iid = "c0f10001-0000-4000-8000-000000000001")]
pub unsafe trait IComFixtureFirst {
    /// Read the first value.
    fn FirstValue(&self) -> u32;
}

/// A second COM interface with its own `IUnknown` prefix.
#[interface(abi = com, iid = "c0f10002-0000-4000-8000-000000000002")]
pub unsafe trait IComFixtureSecond {
    /// Read the second value.
    fn SecondValue(&self) -> u32;
}

#[implement(IComFixtureFirst, IComFixtureSecond)]
struct RustComFixture {
    drops: Arc<AtomicU32>,
}

// SAFETY: Default standalone hooks, no auxiliary interface pointers or thread contract.
unsafe impl RefCounted for RustComFixture {
    type Policy = SingleRefCount;
}

impl IComFixtureFirstImpl for RustComFixture {
    fn FirstValue(&self) -> u32 {
        41
    }
}

impl IComFixtureSecondImpl for RustComFixture {
    fn SecondValue(&self) -> u32 {
        99
    }
}

impl Drop for RustComFixture {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
    }
}

cpp! {{
    #include <cstdint>
    #include <cstring>

    #if defined(_WIN32) && (defined(_M_IX86) || defined(__i386__))
    #define CPPVTABLE_COM_FIXTURE_CALL __stdcall
    #else
    #define CPPVTABLE_COM_FIXTURE_CALL
    #endif

    struct CppvtableComFixtureGuid {
        std::uint32_t data1;
        std::uint16_t data2;
        std::uint16_t data3;
        std::uint8_t data4[8];
    };
    static_assert(sizeof(CppvtableComFixtureGuid) == 16, "COM GUID layout");

    const CppvtableComFixtureGuid CppvtableComFixtureUnknownId =
        {0, 0, 0, {0xc0, 0, 0, 0, 0, 0, 0, 0x46}};
    const CppvtableComFixtureGuid CppvtableComFixtureFirstId =
        {0xc0f10001, 0, 0x4000, {0x80, 0, 0, 0, 0, 0, 0, 1}};
    const CppvtableComFixtureGuid CppvtableComFixtureSecondId =
        {0xc0f10002, 0, 0x4000, {0x80, 0, 0, 0, 0, 0, 0, 2}};

    class CppvtableComFixtureUnknown {
    public:
        virtual std::int32_t CPPVTABLE_COM_FIXTURE_CALL QueryInterface(
            const CppvtableComFixtureGuid* iid, void** out) = 0;
        virtual std::uint32_t CPPVTABLE_COM_FIXTURE_CALL AddRef() = 0;
        virtual std::uint32_t CPPVTABLE_COM_FIXTURE_CALL Release() = 0;
    };

    class CppvtableComFixtureFirst : public CppvtableComFixtureUnknown {
    public:
        virtual std::uint32_t CPPVTABLE_COM_FIXTURE_CALL FirstValue() = 0;
    };

    class CppvtableComFixtureSecond : public CppvtableComFixtureUnknown {
    public:
        virtual std::uint32_t CPPVTABLE_COM_FIXTURE_CALL SecondValue() = 0;
    };

    class CppvtableComFixtureNative final :
        public CppvtableComFixtureFirst, public CppvtableComFixtureSecond {
    public:
        std::uint32_t refs;
        std::uint32_t* drops;

        explicit CppvtableComFixtureNative(std::uint32_t* counter) : refs(1), drops(counter) {}
        ~CppvtableComFixtureNative() { ++*drops; }

        std::int32_t CPPVTABLE_COM_FIXTURE_CALL QueryInterface(
            const CppvtableComFixtureGuid* iid, void** out) override {
            if (!out) return static_cast<std::int32_t>(0x80004003u);
            *out = nullptr;
            if (!iid) return static_cast<std::int32_t>(0x80004003u);
            if (std::memcmp(iid, &CppvtableComFixtureFirstId, sizeof(*iid)) == 0 ||
                std::memcmp(iid, &CppvtableComFixtureUnknownId, sizeof(*iid)) == 0) {
                *out = static_cast<CppvtableComFixtureFirst*>(this);
            } else if (std::memcmp(iid, &CppvtableComFixtureSecondId, sizeof(*iid)) == 0) {
                *out = static_cast<CppvtableComFixtureSecond*>(this);
            } else {
                return static_cast<std::int32_t>(0x80004002u);
            }
            AddRef();
            return 0;
        }

        std::uint32_t CPPVTABLE_COM_FIXTURE_CALL AddRef() override { return ++refs; }
        std::uint32_t CPPVTABLE_COM_FIXTURE_CALL Release() override {
            const std::uint32_t remaining = --refs;
            if (!remaining) delete this;
            return remaining;
        }
        std::uint32_t CPPVTABLE_COM_FIXTURE_CALL FirstValue() override { return 41; }
        std::uint32_t CPPVTABLE_COM_FIXTURE_CALL SecondValue() override { return 99; }
    };
}}

/// # Safety
/// The pointer must borrow a live primary COM fixture interface with one owned reference.
unsafe fn cpp_exercise_rust_com(pointer: *mut c_void) -> bool {
    cpp!(unsafe [pointer as "CppvtableComFixtureFirst*"] -> bool as "bool" {
        bool valid = pointer->FirstValue() == 41;
        valid = (pointer->AddRef() == 2) && valid;
        valid = (pointer->Release() == 1) && valid;

        void* second_raw = nullptr;
        if (pointer->QueryInterface(&CppvtableComFixtureSecondId, &second_raw) != 0 ||
            !second_raw) return false;
        auto* second = static_cast<CppvtableComFixtureSecond*>(second_raw);
        valid = (second->SecondValue() == 99) && valid;
        valid = (static_cast<void*>(second) != static_cast<void*>(pointer)) && valid;

        void* identity_first = nullptr;
        void* identity_second = nullptr;
        const auto first_status = pointer->QueryInterface(
            &CppvtableComFixtureUnknownId, &identity_first);
        const auto second_status = second->QueryInterface(
            &CppvtableComFixtureUnknownId, &identity_second);
        valid = (first_status == 0 && second_status == 0 &&
            identity_first == pointer && identity_second == pointer) && valid;
        if (identity_first) static_cast<CppvtableComFixtureUnknown*>(identity_first)->Release();
        if (identity_second) static_cast<CppvtableComFixtureUnknown*>(identity_second)->Release();
        valid = (second->Release() == 1) && valid;

        CppvtableComFixtureGuid missing = {};
        void* absent = pointer;
        const auto missing_status = pointer->QueryInterface(&missing, &absent);
        valid = (missing_status == static_cast<std::int32_t>(0x80004002u) &&
            absent == nullptr) && valid;
        return valid;
    })
}

/// # Safety
/// The aligned writable drop counter must outlive every reference to the returned object.
unsafe fn cpp_create_native_com(drops: *mut u32) -> *mut c_void {
    cpp!(unsafe [drops as "std::uint32_t*"] -> *mut c_void as "void*" {
        return static_cast<CppvtableComFixtureFirst*>(new CppvtableComFixtureNative(drops));
    })
}

/// # Safety
/// The pointer must borrow a live primary interface allocated by the native fixture factory.
unsafe fn cpp_native_public_count(pointer: *mut c_void) -> u32 {
    cpp!(unsafe [pointer as "CppvtableComFixtureFirst*"] -> u32 as "std::uint32_t" {
        return static_cast<CppvtableComFixtureNative*>(pointer)->refs;
    })
}

/// # Safety
/// The pointer must own a live COM reference consumed by this call.
unsafe fn cpp_release_com(pointer: *mut c_void) -> u32 {
    cpp!(unsafe [pointer as "CppvtableComFixtureUnknown*"] -> u32 as "std::uint32_t" {
        return pointer->Release();
    })
}

#[test]
fn cpp_calls_rust_com_query_interface_and_reference_counting() {
    let drops = Arc::new(AtomicU32::new(0));
    let pointer = ComObject::new(RustComFixture {
        drops: Arc::clone(&drops),
    });
    // SAFETY: The owning pointer remains alive during the native calls and owns count 1.
    assert!(unsafe { cpp_exercise_rust_com(pointer.as_raw()) });
    assert_eq!(pointer.public_count_of::<RustComFixture>(), Some(1));
    assert_eq!(drops.load(Ordering::Relaxed), 0);
    let raw = pointer.into_raw();
    // SAFETY: The owning handle transferred its final reference to the native caller.
    assert_eq!(unsafe { cpp_release_com(raw) }, 0);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn rust_calls_native_com_clone_cast_identity_and_final_release() {
    use cppvtable_com::IUnknown;

    let mut drops = 0_u32;
    // SAFETY: The local drop counter outlives all references; the factory owns count 1.
    let pointer = unsafe {
        ComPtr::<IComFixtureFirst>::from_raw_unchecked(cpp_create_native_com(&raw mut drops))
    };
    // SAFETY: The native object is alive and its methods take no raw arguments.
    assert_eq!(unsafe { pointer.FirstValue() }, 41);
    let copy = pointer.clone();
    // SAFETY: `pointer` still owns a live native primary interface.
    assert_eq!(unsafe { cpp_native_public_count(pointer.as_raw()) }, 2);
    let second = pointer.cast::<IComFixtureSecond>().unwrap();
    assert_ne!(second.as_raw(), pointer.as_raw());
    // SAFETY: The secondary owning pointer keeps the object alive.
    assert_eq!(unsafe { second.SecondValue() }, 99);
    let first_identity = pointer.cast::<IUnknown>().unwrap();
    let second_identity = second.cast::<IUnknown>().unwrap();
    assert_eq!(first_identity.as_raw(), pointer.as_raw());
    assert_eq!(second_identity, first_identity);
    // SAFETY: All five references refer to the live native object.
    assert_eq!(unsafe { cpp_native_public_count(pointer.as_raw()) }, 5);
    drop((copy, second, first_identity, second_identity));
    // SAFETY: The original pointer owns the last live native reference.
    assert_eq!(unsafe { cpp_native_public_count(pointer.as_raw()) }, 1);
    assert_eq!(drops, 0);
    drop(pointer);
    assert_eq!(drops, 1);
}
