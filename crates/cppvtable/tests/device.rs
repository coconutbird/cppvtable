//! The reference model of Direct3D 9: a device and a resource.
//!
//! The rules of section 5.1 of the design document:
//!
//! - The public count of a resource holds one public reference of the device. A live
//!   resource therefore keeps the device alive.
//! - The device holds a private reference of each bound resource. A resource that the
//!   application released stays alive while it is bound, so `GetTexture` gives back the
//!   same pointer that `SetTexture` got.
//! - A private reference never keeps the device alive, so no cycle exists.
//! - When the public count of the device goes to zero, the device clears its state. That
//!   releases the private references.
//!
//! The tests release the device and the resource in both orders and check that each
//! object is destroyed exactly one time.

use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cppvtable::{
    ComObject, ComPtr, E_POINTER, HRESULT, PrivateRef, RefCounted, S_FALSE, S_OK, implement,
    interface, unknown_add_ref, unknown_release,
};

/// The device interface.
#[interface(abi = com, iid = "d3d90001-0000-4000-8000-000000000001")]
pub unsafe trait IDevice9 {
    /// Bind a texture to a stage. A null pointer clears the stage.
    fn SetTexture(&self, stage: u32, texture: *mut c_void) -> HRESULT;
    /// Give the bound texture of a stage. The call adds a public reference.
    fn GetTexture(&self, stage: u32, texture: *mut *mut c_void) -> HRESULT;
}

/// The base interface of a resource.
#[interface(abi = com, iid = "d3d90002-0000-4000-8000-000000000002")]
pub unsafe trait IResource9 {
    /// Give the device of the resource. The call adds a public reference.
    fn GetDevice(&self, device: *mut *mut c_void) -> HRESULT;
}

/// A texture.
#[interface(abi = com, iid = "d3d90003-0000-4000-8000-000000000003", extends(IResource9))]
pub unsafe trait ITexture9 {
    /// Give the number of levels.
    fn GetLevelCount(&self) -> u32;
}

/// The counters of the test.
#[derive(Default)]
struct Counters {
    /// The number of device destructions.
    device_drops: AtomicU32,
    /// The number of texture destructions.
    texture_drops: AtomicU32,
    /// The number of `on_first_public_ref` calls of the texture.
    texture_first: AtomicU32,
    /// The number of `on_last_public_release` calls of the texture.
    texture_last: AtomicU32,
}

/// The device object.
#[implement(IDevice9)]
struct Device {
    /// The bound texture of stage 0. The device holds a private reference.
    bound: Mutex<Option<PrivateRef<Texture>>>,
    /// The counters of the test.
    counters: Arc<Counters>,
}

impl RefCounted for Device {
    type Policy = cppvtable::DualRefCount;

    fn on_last_public_release(&self) {
        // The device clears its state. This releases the private references. The lock
        // is free again before the private reference goes away, because a destructor
        // must not run while the device holds the lock.
        let bound = self.bound.lock().unwrap().take();
        drop(bound);
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.counters.device_drops.fetch_add(1, Ordering::Relaxed);
    }
}

impl IDevice9Impl for Device {
    fn SetTexture(&self, stage: u32, texture: *mut c_void) -> HRESULT {
        if stage != 0 {
            return E_POINTER;
        }
        // SAFETY: The application gives a valid interface pointer or a null pointer.
        let new = unsafe { PrivateRef::<Texture>::from_raw(texture) };
        if !texture.is_null() && new.is_none() {
            // The pointer belongs to another process or to another implementation.
            return E_POINTER;
        }
        let old = core::mem::replace(&mut *self.bound.lock().unwrap(), new);
        drop(old);
        S_OK
    }

    fn GetTexture(&self, stage: u32, texture: *mut *mut c_void) -> HRESULT {
        if texture.is_null() {
            return E_POINTER;
        }
        // SAFETY: The pointer is not null and the application gives a writable place.
        unsafe { *texture = ptr::null_mut() };
        if stage != 0 {
            return E_POINTER;
        }
        let bound = self.bound.lock().unwrap().clone();
        let Some(bound) = bound else {
            return S_FALSE;
        };
        let public: ComPtr<ITexture9> = bound.to_public();
        // SAFETY: The pointer is not null and the application gives a writable place.
        unsafe { *texture = public.into_raw() };
        S_OK
    }
}

/// The texture object.
#[implement(ITexture9)]
struct Texture {
    /// The interface pointer of the device. The texture does not own a reference. The
    /// public count of the texture owns one instead.
    device: AtomicPtr<c_void>,
    /// The number of levels.
    levels: u32,
    /// The counters of the test.
    counters: Arc<Counters>,
}

impl Texture {
    /// Give the interface pointer of the device.
    fn device(&self) -> *mut c_void {
        self.device.load(Ordering::Relaxed)
    }
}

impl RefCounted for Texture {
    type Policy = cppvtable::DualRefCount;

    fn on_first_public_ref(&self) {
        self.counters.texture_first.fetch_add(1, Ordering::Relaxed);
        // The Direct3D 9 rule: a live resource keeps the device alive.
        // SAFETY: The device is alive. Either the device holds a private reference of
        // this texture, or this texture holds a public reference of the device.
        unsafe { unknown_add_ref(self.device()) };
    }

    fn on_last_public_release(&self) {
        self.counters.texture_last.fetch_add(1, Ordering::Relaxed);
        // SAFETY: This texture owns the public reference of the device that the call
        // removes.
        unsafe { unknown_release(self.device()) };
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        self.counters.texture_drops.fetch_add(1, Ordering::Relaxed);
    }
}

impl IResource9Impl for Texture {
    fn GetDevice(&self, device: *mut *mut c_void) -> HRESULT {
        if device.is_null() {
            return E_POINTER;
        }
        let pointer = self.device();
        // SAFETY: The device is alive while this texture has a public reference.
        unsafe { unknown_add_ref(pointer) };
        // SAFETY: The pointer is not null and the application gives a writable place.
        unsafe { *device = pointer };
        S_OK
    }
}

impl ITexture9Impl for Texture {
    fn GetLevelCount(&self) -> u32 {
        self.levels
    }
}

/// Make a device and a texture of that device.
fn new_device_and_texture(counters: &Arc<Counters>) -> (ComPtr<IDevice9>, ComPtr<ITexture9>) {
    let device = ComObject::new(Device {
        bound: Mutex::new(None),
        counters: Arc::clone(counters),
    });
    let texture = ComObject::new(Texture {
        device: AtomicPtr::new(device.as_raw()),
        levels: 3,
        counters: Arc::clone(counters),
    });
    (device, texture)
}

#[test]
fn a_live_resource_keeps_the_device_alive() {
    let counters = Arc::new(Counters::default());
    let (device, texture) = new_device_and_texture(&counters);

    // The new texture took one public reference of the device.
    assert_eq!(device.public_count_of::<Device>(), Some(2));
    assert_eq!(counters.texture_first.load(Ordering::Relaxed), 1);

    // `GetDevice` gives one more public reference.
    let mut raw: *mut c_void = ptr::null_mut();
    // SAFETY: The object is alive and `raw` is a local value.
    assert!(unsafe { texture.GetDevice(&raw mut raw) }.is_ok());
    assert_eq!(raw, device.as_raw());
    // SAFETY: `GetDevice` added the reference that this `ComPtr` owns.
    let from_texture = unsafe { ComPtr::<IDevice9>::from_raw(raw) }.unwrap();
    assert_eq!(device.public_count_of::<Device>(), Some(3));
    drop(from_texture);

    // The application releases the device. The texture keeps it alive.
    let device_raw = device.as_raw();
    drop(device);
    assert_eq!(counters.device_drops.load(Ordering::Relaxed), 0);
    // SAFETY: The object is alive.
    assert_eq!(unsafe { texture.GetLevelCount() }, 3);

    drop(texture);
    assert_eq!(counters.texture_drops.load(Ordering::Relaxed), 1);
    assert_eq!(counters.device_drops.load(Ordering::Relaxed), 1);
    let _ = device_raw;
}

#[test]
fn a_bound_resource_stays_alive_and_keeps_its_address() {
    let counters = Arc::new(Counters::default());
    let (device, texture) = new_device_and_texture(&counters);
    let address = texture.as_raw();

    // SAFETY: The two objects are alive.
    assert!(unsafe { device.SetTexture(0, address) }.is_ok());

    // The application releases the texture. The device holds a private reference, so
    // the texture stays alive. Its public count is zero, so it gave the public
    // reference of the device back.
    drop(texture);
    assert_eq!(counters.texture_drops.load(Ordering::Relaxed), 0);
    assert_eq!(counters.texture_last.load(Ordering::Relaxed), 1);
    assert_eq!(device.public_count_of::<Device>(), Some(1));

    // `GetTexture` gives the same pointer back and brings the public count to 1.
    let mut raw: *mut c_void = ptr::null_mut();
    // SAFETY: The device is alive and `raw` is a local value.
    assert!(unsafe { device.GetTexture(0, &raw mut raw) }.is_ok());
    assert_eq!(raw, address);
    assert_eq!(counters.texture_first.load(Ordering::Relaxed), 2);
    assert_eq!(device.public_count_of::<Device>(), Some(2));
    // SAFETY: `GetTexture` added the reference that this `ComPtr` owns.
    let again = unsafe { ComPtr::<ITexture9>::from_raw(raw) }.unwrap();
    // SAFETY: The object is alive.
    assert_eq!(unsafe { again.GetLevelCount() }, 3);

    drop(again);
    drop(device);
    assert_eq!(counters.texture_drops.load(Ordering::Relaxed), 1);
    assert_eq!(counters.device_drops.load(Ordering::Relaxed), 1);
}

#[test]
fn the_release_of_the_texture_first_destroys_both_objects_one_time() {
    let counters = Arc::new(Counters::default());
    let (device, texture) = new_device_and_texture(&counters);
    // SAFETY: The two objects are alive.
    assert!(unsafe { device.SetTexture(0, texture.as_raw()) }.is_ok());

    drop(texture);
    assert_eq!(counters.texture_drops.load(Ordering::Relaxed), 0);
    drop(device);

    assert_eq!(counters.texture_drops.load(Ordering::Relaxed), 1);
    assert_eq!(counters.device_drops.load(Ordering::Relaxed), 1);
    assert_eq!(
        counters.texture_first.load(Ordering::Relaxed),
        counters.texture_last.load(Ordering::Relaxed)
    );
}

#[test]
fn the_release_of_the_device_first_destroys_both_objects_one_time() {
    let counters = Arc::new(Counters::default());
    let (device, texture) = new_device_and_texture(&counters);
    // SAFETY: The two objects are alive.
    assert!(unsafe { device.SetTexture(0, texture.as_raw()) }.is_ok());

    // The device has two public references: one of the application and one of the
    // texture. The release of the application does not destroy it.
    drop(device);
    assert_eq!(counters.device_drops.load(Ordering::Relaxed), 0);
    assert_eq!(counters.texture_drops.load(Ordering::Relaxed), 0);

    // The release of the texture removes the last public reference of the device. The
    // device then clears its state, which releases the private reference of the
    // texture.
    drop(texture);
    assert_eq!(counters.texture_drops.load(Ordering::Relaxed), 1);
    assert_eq!(counters.device_drops.load(Ordering::Relaxed), 1);
    assert_eq!(
        counters.texture_first.load(Ordering::Relaxed),
        counters.texture_last.load(Ordering::Relaxed)
    );
}

#[test]
fn set_texture_refuses_a_pointer_of_another_implementation() {
    let counters = Arc::new(Counters::default());
    let (device, texture) = new_device_and_texture(&counters);

    // A null pointer clears the stage.
    // SAFETY: The device is alive.
    assert!(unsafe { device.SetTexture(0, ptr::null_mut()) }.is_ok());
    let mut raw: *mut c_void = ptr::null_mut();
    // SAFETY: The device is alive and `raw` is a local value.
    assert_eq!(unsafe { device.GetTexture(0, &raw mut raw) }, S_FALSE);
    assert!(raw.is_null());

    // The pointer of the device is not a texture.
    // SAFETY: The device is alive.
    let wrong_type = unsafe { device.SetTexture(0, device.as_raw()) };
    assert_eq!(wrong_type, E_POINTER);

    // A null out-pointer gives `E_POINTER`.
    // SAFETY: The device is alive. A null out-pointer is the case that the test checks.
    assert_eq!(unsafe { device.GetTexture(0, ptr::null_mut()) }, E_POINTER);

    drop(texture);
    drop(device);
    assert_eq!(counters.texture_drops.load(Ordering::Relaxed), 1);
    assert_eq!(counters.device_drops.load(Ordering::Relaxed), 1);
}
