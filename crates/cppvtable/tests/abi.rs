//! The three binary interfaces and the aggregate return value.
//!
//! - `abi = com` uses `extern "system"`: stdcall on x86 and the C convention on every
//!   other target.
//! - `abi = cpp` uses `extern "thiscall"` on x86 and `extern "C"` on every other target.
//!   The vtable of such an interface has no `IUnknown` slots.
//! - `abi = c` uses `extern "C"` on every target.
//!
//! `#[abi(hidden_return)]` makes the shim of the MSVC rule for a structure return value:
//! the hidden pointer is the first argument after `this`, and the function gives that
//! pointer back.

use core::ffi::c_void;
use core::mem::{offset_of, size_of};
use core::sync::atomic::{AtomicI32, Ordering};

use cppvtable::{
    ComObject, ComPtr, HRESULT, Interface, OwnedObject, RefCounted, S_OK, SingleRefCount,
    implement, interface, interface_of,
};

/// A three-element vector. MSVC gives it back through a hidden pointer.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vector3 {
    /// The first element.
    pub x: i32,
    /// The second element.
    pub y: i32,
    /// The third element.
    pub z: i32,
}

/// A transparent wrapper of a number. MSVC gives it back in a register.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag(pub u32);

/// A COM interface with aggregate return values.
#[interface(abi = com, iid = "ab100001-0000-4000-8000-000000000001")]
pub unsafe trait IGeometry {
    /// Give the origin. The value goes back through a hidden pointer.
    #[abi(hidden_return)]
    fn GetOrigin(&self) -> Vector3;
    /// Give the origin, multiplied by the factor.
    #[abi(hidden_return)]
    fn GetScaled(&self, factor: i32) -> Vector3;
    /// Give the tag. The value goes back in a register.
    #[abi(scalar)]
    fn GetTag(&self) -> Tag;
    /// Set the origin.
    fn SetOrigin(&self, x: i32, y: i32, z: i32) -> HRESULT;
    /// A method with the shape of `CreateTexture`: more arguments than the lint
    /// `clippy::too_many_arguments` permits. The macro adds the expectation to each item
    /// that it makes, and the lint never fires on a method of a trait implementation.
    fn CreateThing(
        &self,
        width: u32,
        height: u32,
        levels: u32,
        usage: u32,
        format: u32,
        pool: u32,
        flags: u32,
        extra: u32,
        sum: *mut u32,
    ) -> HRESULT;
}

/// The base of a C++ class.
#[interface(abi = cpp)]
pub unsafe trait ICppBase {
    /// Give the area.
    fn Area(&self) -> u32;
}

/// A C++ class with a base class.
#[interface(abi = cpp, extends(ICppBase))]
pub unsafe trait ICppShape {
    /// Multiply the size by the factor and give the new area.
    fn Scale(&self, factor: u32) -> u32;
}

/// A C table of function pointers with a `this` argument.
#[interface(abi = c, iid = "ab100002-0000-4000-8000-000000000002")]
pub unsafe trait ICValue {
    /// Give the value.
    fn get_value(&self) -> i32;
    /// Set the value.
    fn set_value(&self, value: i32);
}

/// An object with a COM interface, a C++ class, and a C table.
#[implement(IGeometry, ICppShape, ICValue)]
struct Shape {
    /// The first element of the origin.
    x: AtomicI32,
    /// The second element of the origin.
    y: AtomicI32,
    /// The third element of the origin.
    z: AtomicI32,
    /// The size of the C++ shape.
    size: AtomicI32,
}

impl RefCounted for Shape {
    type Policy = SingleRefCount;
}

impl IGeometryImpl for Shape {
    fn GetOrigin(&self) -> Vector3 {
        Vector3 {
            x: self.x.load(Ordering::Relaxed),
            y: self.y.load(Ordering::Relaxed),
            z: self.z.load(Ordering::Relaxed),
        }
    }

    fn GetScaled(&self, factor: i32) -> Vector3 {
        let origin = self.GetOrigin();
        Vector3 {
            x: origin.x * factor,
            y: origin.y * factor,
            z: origin.z * factor,
        }
    }

    fn GetTag(&self) -> Tag {
        Tag(0xfeed_0001)
    }

    fn SetOrigin(&self, x: i32, y: i32, z: i32) -> HRESULT {
        self.x.store(x, Ordering::Relaxed);
        self.y.store(y, Ordering::Relaxed);
        self.z.store(z, Ordering::Relaxed);
        S_OK
    }

    fn CreateThing(
        &self,
        width: u32,
        height: u32,
        levels: u32,
        usage: u32,
        format: u32,
        pool: u32,
        flags: u32,
        extra: u32,
        sum: *mut u32,
    ) -> HRESULT {
        if sum.is_null() {
            return cppvtable::E_POINTER;
        }
        // SAFETY: The pointer is not null, and the caller gives a writable place.
        unsafe { *sum = width + height + levels + usage + format + pool + flags + extra };
        S_OK
    }
}

impl ICppBaseImpl for Shape {
    fn Area(&self) -> u32 {
        let size = self.size.load(Ordering::Relaxed);
        u32::try_from(size * size).unwrap_or(0)
    }
}

impl ICppShapeImpl for Shape {
    fn Scale(&self, factor: u32) -> u32 {
        let factor = i32::try_from(factor).unwrap_or(1);
        let size = self.size.load(Ordering::Relaxed);
        self.size.store(size * factor, Ordering::Relaxed);
        self.Area()
    }
}

impl ICValueImpl for Shape {
    fn get_value(&self) -> i32 {
        self.size.load(Ordering::Relaxed)
    }

    fn set_value(&self, value: i32) {
        self.size.store(value, Ordering::Relaxed);
    }
}

/// Make a new shape.
fn new_shape() -> ComPtr<IGeometry> {
    ComObject::new(Shape {
        x: AtomicI32::new(1),
        y: AtomicI32::new(2),
        z: AtomicI32::new(3),
        size: AtomicI32::new(4),
    })
}

#[test]
fn a_cpp_vtable_has_no_iunknown_slots() {
    let pointer = size_of::<usize>();
    assert_eq!(size_of::<ICppBaseVtbl>(), pointer);
    assert_eq!(size_of::<ICppShapeVtbl>(), 2 * pointer);
    assert_eq!(offset_of!(ICppShapeVtbl, base), 0);
    assert_eq!(offset_of!(ICppShapeVtbl, Scale), pointer);

    assert_eq!(size_of::<ICValueVtbl>(), 2 * pointer);
    assert_eq!(offset_of!(ICValueVtbl, get_value), 0);
    assert_eq!(offset_of!(ICValueVtbl, set_value), pointer);
}

#[test]
fn the_com_vtable_holds_one_slot_for_each_method() {
    let pointer = size_of::<usize>();
    assert_eq!(size_of::<IGeometryVtbl>(), 8 * pointer);
    assert_eq!(offset_of!(IGeometryVtbl, CreateThing), 7 * pointer);
    assert_eq!(offset_of!(IGeometryVtbl, GetOrigin), 3 * pointer);
    assert_eq!(offset_of!(IGeometryVtbl, GetScaled), 4 * pointer);
    assert_eq!(offset_of!(IGeometryVtbl, GetTag), 5 * pointer);
    assert_eq!(offset_of!(IGeometryVtbl, SetOrigin), 6 * pointer);
}

#[test]
fn a_hidden_return_shim_writes_to_the_hidden_pointer_and_gives_it_back() {
    let shape = new_shape();
    let this = shape.as_raw();
    // SAFETY: `this` is a valid interface pointer of `IGeometry`.
    let vtable = unsafe { *this.cast::<*const IGeometryVtbl>() };

    let mut result = Vector3 { x: 0, y: 0, z: 0 };
    // SAFETY: The vtable belongs to the object and `result` is a local value.
    let given = unsafe { ((*vtable).GetOrigin)(this, &raw mut result) };
    assert_eq!(given, &raw mut result);
    assert_eq!(result, Vector3 { x: 1, y: 2, z: 3 });

    let mut scaled = Vector3 { x: 0, y: 0, z: 0 };
    // SAFETY: The vtable belongs to the object and `scaled` is a local value.
    let given = unsafe { ((*vtable).GetScaled)(this, &raw mut scaled, 3) };
    assert_eq!(given, &raw mut scaled);
    assert_eq!(scaled, Vector3 { x: 3, y: 6, z: 9 });

    // The safe wrapper hides the pointer.
    // SAFETY: The object is alive.
    unsafe {
        assert_eq!(shape.GetOrigin(), Vector3 { x: 1, y: 2, z: 3 });
        assert_eq!(shape.GetScaled(2), Vector3 { x: 2, y: 4, z: 6 });
        assert_eq!(shape.GetTag(), Tag(0xfeed_0001));
        assert!(shape.SetOrigin(7, 8, 9).is_ok());
        assert_eq!(shape.GetOrigin(), Vector3 { x: 7, y: 8, z: 9 });
    }
}

#[test]
fn a_method_with_many_arguments_works_through_the_vtable() {
    let shape = new_shape();
    let this = shape.as_raw();
    // SAFETY: `this` is a valid interface pointer of `IGeometry`.
    let vtable = unsafe { *this.cast::<*const IGeometryVtbl>() };

    let mut sum = 0_u32;
    // SAFETY: The vtable belongs to the object and `sum` is a local value.
    let result = unsafe { ((*vtable).CreateThing)(this, 1, 2, 3, 4, 5, 6, 7, 8, &raw mut sum) };
    assert!(result.is_ok());
    assert_eq!(sum, 36);

    let mut again = 0_u32;
    // SAFETY: The object is alive and `again` is a local value.
    let result = unsafe { shape.CreateThing(1, 1, 1, 1, 1, 1, 1, 1, &raw mut again) };
    assert!(result.is_ok());
    assert_eq!(again, 8);
}

#[test]
fn a_cpp_caller_reaches_the_object_through_the_second_vtable_pointer() {
    let shape = new_shape();
    let value = shape.as_impl::<Shape>().unwrap();
    // SAFETY: `value` is the Rust value inside a live object.
    let cpp = unsafe { interface_of::<Shape, ICppShape>(value) };
    assert_eq!(
        cpp as usize - shape.as_raw() as usize,
        size_of::<*const c_void>()
    );

    // SAFETY: `cpp` is a valid interface pointer of `ICppShape`.
    let vtable = unsafe { *cpp.cast::<*const ICppShapeVtbl>() };
    // SAFETY: The vtable belongs to the object. The `this` adjustment of the shim moves
    // the pointer back to the start of the allocation.
    assert_eq!(unsafe { ((*vtable).base.Area)(cpp) }, 16);
    // SAFETY: The vtable belongs to the object.
    assert_eq!(unsafe { ((*vtable).Scale)(cpp, 2) }, 64);
}

#[test]
fn a_c_caller_reaches_the_object_through_the_third_vtable_pointer() {
    let shape = new_shape();
    let value = shape.as_impl::<Shape>().unwrap();
    // SAFETY: `value` is the Rust value inside a live object.
    let table = unsafe { interface_of::<Shape, ICValue>(value) };
    assert_eq!(
        table as usize - shape.as_raw() as usize,
        2 * size_of::<*const c_void>()
    );

    // SAFETY: `table` is a valid interface pointer of `ICValue`.
    let vtable = unsafe { *table.cast::<*const ICValueVtbl>() };
    // SAFETY: The vtable belongs to the object.
    assert_eq!(unsafe { ((*vtable).get_value)(table) }, 4);
    // SAFETY: The vtable belongs to the object.
    unsafe { ((*vtable).set_value)(table, 11) };
    // SAFETY: The vtable belongs to the object.
    assert_eq!(unsafe { ((*vtable).get_value)(table) }, 11);
    assert_eq!(value.size.load(Ordering::Relaxed), 11);
}

#[test]
fn query_interface_does_not_answer_a_cpp_or_c_interface() {
    let shape = new_shape();
    assert!(shape.cast::<ICppShape>().is_none());
    assert!(shape.cast::<ICValue>().is_none());
    assert!(!cppvtable::interface_matches::<ICValue>(&ICValue::IID));
}

/// A plain C++ object. No container owns it, so `AddRef` and `Release` do nothing.
#[implement(ICppShape)]
struct PlainShape {
    /// The size of the shape.
    size: AtomicI32,
}

impl RefCounted for PlainShape {
    type Policy = cppvtable::ForwardRefCount;
}

impl ICppBaseImpl for PlainShape {
    fn Area(&self) -> u32 {
        let size = self.size.load(Ordering::Relaxed);
        u32::try_from(size * size).unwrap_or(0)
    }
}

impl ICppShapeImpl for PlainShape {
    fn Scale(&self, factor: u32) -> u32 {
        let factor = i32::try_from(factor).unwrap_or(1);
        let size = self.size.load(Ordering::Relaxed);
        self.size.store(size * factor, Ordering::Relaxed);
        self.Area()
    }
}

#[test]
fn a_rust_owner_can_hold_a_plain_cpp_object() {
    let owner = OwnedObject::new(PlainShape {
        size: AtomicI32::new(3),
    });
    let this = owner.as_raw::<ICppShape>();
    // SAFETY: `this` is a valid interface pointer of `ICppShape`.
    let vtable = unsafe { *this.cast::<*const ICppShapeVtbl>() };
    // SAFETY: The vtable belongs to the object.
    assert_eq!(unsafe { ((*vtable).base.Area)(this) }, 9);
    assert_eq!(owner.get().size.load(Ordering::Relaxed), 3);
}
