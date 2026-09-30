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
    };
}}

fn create_cpp_returns(bias: i64) -> *mut c_void {
    cpp!(unsafe [bias as "std::int64_t"] -> *mut c_void as "void*" {
        return new CppReturns(bias);
    })
}

fn delete_cpp_returns(object: *mut c_void) {
    cpp!(unsafe [object as "CppReturns*"] { delete object; });
}

fn cpp_small(object: *mut c_void, value: i32) -> Small {
    cpp!(unsafe [object as "IReturns*", value as "std::int32_t"] -> Small as "Small" {
        return object->small(value);
    })
}

fn cpp_large(object: *mut c_void, value: i64) -> Large {
    cpp!(unsafe [object as "IReturns*", value as "std::int64_t"] -> Large as "Large" {
        return object->large(value);
    })
}

fn cpp_floating(object: *mut c_void, value: f64) -> f64 {
    cpp!(unsafe [object as "IReturns*", value as "double"] -> f64 as "double" {
        return object->floating(value);
    })
}

fn cpp_pointer(object: *mut c_void, value: *mut c_void) -> *mut c_void {
    cpp!(unsafe [object as "IReturns*", value as "void*"] -> *mut c_void as "void*" {
        return object->pointer(value);
    })
}

fn cpp_wide(object: *mut c_void, value: i64) -> i64 {
    cpp!(unsafe [object as "IReturns*", value as "std::int64_t"] -> i64 as "std::int64_t" {
        return object->wide(value);
    })
}

fn cpp_indirect(object: *mut c_void, value: i64) -> Large {
    cpp!(unsafe [object as "IReturns*", value as "std::int64_t"] -> Large as "Large" {
        return object->indirect(value);
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
    }
    delete_cpp_returns(raw);
}

#[test]
fn cpp_calls_rust_return_abis() {
    let owner = OwnedObject::new(Returns { bias: 40 });
    let raw = owner.as_raw::<IReturns>();
    assert_eq!(cpp_small(raw, 9), Small { x: 9, y: 16 });
    assert_eq!(cpp_large(raw, 2), Large { x: 2, y: 40, z: 42 });
    assert_eq!(cpp_floating(raw, 3.25).to_bits(), 3.75_f64.to_bits());
    assert_eq!(cpp_pointer(raw, raw), raw);
    assert_eq!(cpp_wide(raw, 0x1_0000_0000), 0x1_0000_0028);
    assert_eq!(cpp_indirect(raw, 2), Large { x: 2, y: 40, z: 42 });
}
