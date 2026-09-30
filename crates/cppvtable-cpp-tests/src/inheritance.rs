//! C++ interface inheritance and explicit compiler ABI selection.

use cpp::cpp;
#[cfg(test)]
use cppvtable::OwnedObject;
use cppvtable::{implement, interface};
use std::ffi::c_void;

#[interface(abi = cpp)]
unsafe trait IBase {
    fn value(&self) -> i32;
}

#[interface(abi = cpp, extends(IBase))]
unsafe trait IDerived {
    fn scaled(&self, factor: i32) -> i32;
}

#[cfg(target_env = "msvc")]
#[interface(abi = msvc)]
unsafe trait IExplicit {
    fn value(&self) -> i32;
    fn scaled(&self, factor: i32) -> i32;
}

#[cfg(not(target_env = "msvc"))]
#[interface(abi = itanium)]
unsafe trait IExplicit {
    fn value(&self) -> i32;
    fn scaled(&self, factor: i32) -> i32;
}

#[implement(IDerived, IExplicit)]
struct Derived {
    value: i32,
}
impl IBaseImpl for Derived {
    fn value(&self) -> i32 {
        self.value
    }
}
impl IDerivedImpl for Derived {
    fn scaled(&self, factor: i32) -> i32 {
        self.value * factor
    }
}
impl IExplicitImpl for Derived {
    fn value(&self) -> i32 {
        self.value
    }
    fn scaled(&self, factor: i32) -> i32 {
        self.value * factor
    }
}

cpp! {{
    class IBase {
    public:
        virtual int value() = 0;
    };
    class IDerived : public IBase {
    public:
        virtual int scaled(int factor) = 0;
    };
    class CppDerived final : public IDerived {
        int number;
    public:
        explicit CppDerived(int value) : number(value) {}
        int value() override { return number; }
        int scaled(int factor) override { return number * factor; }
    };
}}

fn create_derived(value: i32) -> *mut c_void {
    cpp!(unsafe [value as "int"] -> *mut c_void as "void*" {
        return new CppDerived(value);
    })
}
/// # Safety
/// The pointer must identify the matching live C++ concrete allocation, owned by the caller.
unsafe fn delete_derived(object: *mut c_void) {
    cpp!(unsafe [object as "CppDerived*"] { delete object; });
}
/// # Safety
/// The object pointer must identify the matching live C++ interface or concrete object.
unsafe fn cpp_value(object: *mut c_void) -> i32 {
    cpp!(unsafe [object as "IBase*"] -> i32 as "int" { return object->value(); })
}
/// # Safety
/// The object pointer must identify the matching live C++ interface or concrete object.
unsafe fn cpp_scaled(object: *mut c_void, factor: i32) -> i32 {
    cpp!(unsafe [object as "IDerived*", factor as "int"] -> i32 as "int" {
        return object->scaled(factor);
    })
}

#[test]
fn rust_calls_cpp_inherited_and_explicit_interfaces() {
    let raw = create_derived(11);
    // SAFETY: All declarations have identical slots to C++ IDerived and it stays alive.
    unsafe {
        let derived = IDerived::from_raw_ref(&raw);
        assert_eq!(derived.value(), 11);
        assert_eq!(derived.scaled(3), 33);
        let explicit = IExplicit::from_raw_ref(&raw);
        assert_eq!(explicit.value(), 11);
        assert_eq!(explicit.scaled(3), 33);
    }
    // SAFETY: The pointer is the unique concrete allocation from the factory.
    unsafe { delete_derived(raw) };
}

#[test]
fn cpp_calls_rust_inherited_and_explicit_interfaces() {
    let owner = OwnedObject::new(Derived { value: 11 });
    // SAFETY: The owner keeps both matching interfaces alive through native calls.
    unsafe {
        assert_eq!(cpp_value(owner.as_raw::<IDerived>()), 11);
        assert_eq!(cpp_scaled(owner.as_raw::<IDerived>(), 3), 33);
        assert_eq!(cpp_value(owner.as_raw::<IExplicit>()), 11);
        assert_eq!(cpp_scaled(owner.as_raw::<IExplicit>(), 3), 33);
    }
    let base = owner.query_interface::<IBase>().expect("inherited base");
    assert_eq!(base.as_raw(), owner.as_raw::<IDerived>());
    // SAFETY: The owner's interface remains alive.
    assert_eq!(unsafe { base.value() }, 11);
}
