//! Aggregate and scalar return ABI tests against real C++ virtual calls.

use cpp::cpp;
#[cfg(test)]
use cppvtable::OwnedObject;
use cppvtable::{implement, interface};
use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Small {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Large {
    x: i64,
    y: i64,
    z: i64,
}

#[interface(abi = cpp)]
unsafe trait IReturns {
    #[abi(aggregate)]
    fn small(&self, value: i32) -> Small;
    #[abi(aggregate)]
    fn large(&self, value: i64) -> Large;
    fn floating(&self, value: f64) -> f64;
    fn pointer(&self, value: *mut c_void) -> *mut c_void;
    fn wide(&self, value: i64) -> i64;
    #[abi(hidden_return)]
    fn indirect(&self, value: i64) -> Large;
    /// Return a small aggregate using a value read from caller storage.
    ///
    /// # Safety
    ///
    /// `value` must point to an initialized, aligned `i32` readable for this call.
    #[abi(aggregate)]
    unsafe fn small_from(&self, value: *const i32) -> Small;
    /// Return an indirect aggregate using a value read from caller storage.
    ///
    /// # Safety
    ///
    /// `value` must point to an initialized, aligned `i64` readable for this call.
    #[abi(hidden_return)]
    unsafe fn indirect_from(&self, value: *const i64) -> Large;
}

#[implement(IReturns)]
struct Returns {
    bias: i64,
}

impl IReturnsImpl for Returns {
    fn small(&self, value: i32) -> Small {
        Small {
            x: value,
            y: value + 7,
        }
    }
    fn large(&self, value: i64) -> Large {
        Large {
            x: value,
            y: self.bias,
            z: value + self.bias,
        }
    }
    fn floating(&self, value: f64) -> f64 {
        value + 0.5
    }
    fn pointer(&self, value: *mut c_void) -> *mut c_void {
        value
    }
    fn wide(&self, value: i64) -> i64 {
        value + self.bias
    }
    fn indirect(&self, value: i64) -> Large {
        self.large(value)
    }
    unsafe fn small_from(&self, value: *const i32) -> Small {
        // SAFETY: The implementation contract requires initialized readable storage.
        self.small(unsafe { *value })
    }
    unsafe fn indirect_from(&self, value: *const i64) -> Large {
        // SAFETY: The implementation contract requires initialized readable storage.
        self.large(unsafe { *value })
    }
}

cpp! {{
    #include <cstdint>
    struct Small { std::int32_t x, y; };
    struct Large { std::int64_t x, y, z; };
    class IReturns {
    public:
        virtual Small small(std::int32_t value) = 0;
        virtual Large large(std::int64_t value) = 0;
        virtual double floating(double value) = 0;
        virtual void* pointer(void* value) = 0;
        virtual std::int64_t wide(std::int64_t value) = 0;
        virtual Large indirect(std::int64_t value) = 0;
        virtual Small small_from(const std::int32_t* value) = 0;
        virtual Large indirect_from(const std::int64_t* value) = 0;
    };
    class CppReturns final : public IReturns {
        std::int64_t bias;
    public:
        explicit CppReturns(std::int64_t bias) : bias(bias) {}
        Small small(std::int32_t value) override { return {value, value + 7}; }
        Large large(std::int64_t value) override { return {value, bias, value + bias}; }
        double floating(double value) override { return value + 0.5; }
        void* pointer(void* value) override { return value; }
        std::int64_t wide(std::int64_t value) override { return value + bias; }
        Large indirect(std::int64_t value) override { return large(value); }
        Small small_from(const std::int32_t* value) override { return small(*value); }
        Large indirect_from(const std::int64_t* value) override { return large(*value); }
    };
}}

fn create_cpp_returns(bias: i64) -> *mut c_void {
    cpp!(unsafe [bias as "std::int64_t"] -> *mut c_void as "void*" {
        return new CppReturns(bias);
    })
}

/// # Safety
/// The pointer must identify the matching live C++ concrete allocation, owned by the caller.
unsafe fn delete_cpp_returns(object: *mut c_void) {
    cpp!(unsafe [object as "CppReturns*"] { delete object; });
}

/// # Safety
/// The object pointer must identify the matching live C++ interface or concrete object.
unsafe fn cpp_small(object: *mut c_void, value: i32) -> Small {
    cpp!(unsafe [object as "IReturns*", value as "std::int32_t"] -> Small as "Small" {
        return object->small(value);
    })
}

/// # Safety
/// The object pointer must identify the matching live C++ interface or concrete object.
unsafe fn cpp_large(object: *mut c_void, value: i64) -> Large {
    cpp!(unsafe [object as "IReturns*", value as "std::int64_t"] -> Large as "Large" {
        return object->large(value);
    })
}

/// # Safety
/// The object pointer must identify the matching live C++ interface or concrete object.
unsafe fn cpp_floating(object: *mut c_void, value: f64) -> f64 {
    cpp!(unsafe [object as "IReturns*", value as "double"] -> f64 as "double" {
        return object->floating(value);
    })
}

/// # Safety
/// The object pointer must identify the matching live C++ interface or concrete object.
unsafe fn cpp_pointer(object: *mut c_void, value: *mut c_void) -> *mut c_void {
    cpp!(unsafe [object as "IReturns*", value as "void*"] -> *mut c_void as "void*" {
        return object->pointer(value);
    })
}

/// # Safety
/// The object pointer must identify the matching live C++ interface or concrete object.
unsafe fn cpp_wide(object: *mut c_void, value: i64) -> i64 {
    cpp!(unsafe [object as "IReturns*", value as "std::int64_t"] -> i64 as "std::int64_t" {
        return object->wide(value);
    })
}

/// # Safety
/// The object pointer must identify the matching live C++ interface or concrete object.
unsafe fn cpp_indirect(object: *mut c_void, value: i64) -> Large {
    cpp!(unsafe [object as "IReturns*", value as "std::int64_t"] -> Large as "Large" {
        return object->indirect(value);
    })
}

/// # Safety
/// `object` must be a live `IReturns` and `value` must point to a readable `i32`.
unsafe fn cpp_small_from(object: *mut c_void, value: *const i32) -> Small {
    cpp!(unsafe [object as "IReturns*", value as "const std::int32_t*"] -> Small as "Small" {
        return object->small_from(value);
    })
}

/// # Safety
/// `object` must be a live `IReturns` and `value` must point to a readable `i64`.
unsafe fn cpp_indirect_from(object: *mut c_void, value: *const i64) -> Large {
    cpp!(unsafe [object as "IReturns*", value as "const std::int64_t*"] -> Large as "Large" {
        return object->indirect_from(value);
    })
}

#[test]
fn rust_calls_cpp_return_abis() {
    let raw = create_cpp_returns(40);
    // SAFETY: The C++ object lives through every borrowed virtual call.
    unsafe {
        let interface = IReturns::from_raw_ref(&raw);
        assert_eq!(interface.small(9), Small { x: 9, y: 16 });
        assert_eq!(interface.large(2), Large { x: 2, y: 40, z: 42 });
        assert_eq!(interface.floating(3.25).to_bits(), 3.75_f64.to_bits());
        assert_eq!(interface.pointer(raw), raw);
        assert_eq!(interface.wide(0x1_0000_0000), 0x1_0000_0028);
        assert_eq!(interface.indirect(2), Large { x: 2, y: 40, z: 42 });
        let small_input = 9;
        let large_input = 2;
        assert_eq!(
            interface.small_from(&raw const small_input),
            Small { x: 9, y: 16 }
        );
        assert_eq!(
            interface.indirect_from(&raw const large_input),
            Large { x: 2, y: 40, z: 42 }
        );
    }
    // SAFETY: The pointer is the unique concrete allocation from the factory.
    unsafe { delete_cpp_returns(raw) };
}

#[test]
fn cpp_calls_rust_return_abis() {
    let owner = OwnedObject::new(Returns { bias: 40 });
    let raw = owner.as_raw::<IReturns>();
    let small_input = 9;
    let large_input = 2;
    // SAFETY: The owner keeps the interface alive; local inputs remain readable.
    unsafe {
        assert_eq!(cpp_small(raw, 9), Small { x: 9, y: 16 });
        assert_eq!(cpp_large(raw, 2), Large { x: 2, y: 40, z: 42 });
        assert_eq!(cpp_floating(raw, 3.25).to_bits(), 3.75_f64.to_bits());
        assert_eq!(cpp_pointer(raw, raw), raw);
        assert_eq!(cpp_wide(raw, 0x1_0000_0000), 0x1_0000_0028);
        assert_eq!(cpp_indirect(raw, 2), Large { x: 2, y: 40, z: 42 });
        assert_eq!(
            cpp_small_from(raw, &raw const small_input),
            Small { x: 9, y: 16 }
        );
        assert_eq!(
            cpp_indirect_from(raw, &raw const large_input),
            Large { x: 2, y: 40, z: 42 }
        );
    }
}

#[test]
fn plain_implementation_methods_are_safe_without_an_object_allocation() {
    let implementation = Returns { bias: 40 };
    assert_eq!(implementation.small(9), Small { x: 9, y: 16 });
    assert_eq!(implementation.large(2), Large { x: 2, y: 40, z: 42 });
    assert_eq!(
        implementation.pointer(std::ptr::null_mut()),
        std::ptr::null_mut()
    );
    assert_eq!(implementation.wide(2), 42);
}
