//! The three reference count policies and the hooks.
//!
//! The tests check the order of the hooks, the answers of `Release`, the life of an
//! object that only a private reference holds, and the life of a child object that a
//! container owns.

use core::ffi::c_void;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use cppvtable::{
    ComObject, ComPtr, DualRefCount, ForwardRefCount, IUnknownVtbl, Interface, OwnedObject,
    PrivateRef, RefCounted, SingleRefCount, implement, interface, interface_of,
};

/// A list of the events of the test.
#[derive(Default)]
struct Log(Mutex<Vec<String>>);

impl Log {
    /// Add one event.
    fn push(&self, event: &str) {
        self.0.lock().unwrap().push(event.to_owned());
    }

    /// Take the events and empty the list.
    fn take(&self) -> Vec<String> {
        core::mem::take(&mut self.0.lock().unwrap())
    }
}

/// A small interface for the objects of this test.
#[interface(abi = com, iid = "9c0f0001-0000-4000-8000-000000000001")]
pub unsafe trait IThing {
    /// Give the value of the object.
    fn Value(&self) -> u32;
}

/// An interface of a child object.
#[interface(abi = com, iid = "9c0f0002-0000-4000-8000-000000000002")]
pub unsafe trait IChild {
    /// Give the index of the child inside its container.
    fn Index(&self) -> u32;
}

/// An object with the standard COM reference count.
#[implement(IThing)]
struct Single {
    /// The events of the object.
    log: Arc<Log>,
}

impl RefCounted for Single {
    type Policy = SingleRefCount;

    fn on_first_public_ref(&self) {
        self.log.push("single:first");
    }

    fn on_last_public_release(&self) {
        self.log.push("single:last");
    }
}

impl Drop for Single {
    fn drop(&mut self) {
        self.log.push("single:drop");
    }
}

impl IThingImpl for Single {
    fn Value(&self) -> u32 {
        1
    }
}

/// An object with a public count and a private count.
#[implement(IThing)]
struct Dual {
    /// The events of the object.
    log: Arc<Log>,
}

impl RefCounted for Dual {
    type Policy = DualRefCount;

    fn on_first_public_ref(&self) {
        self.log.push("dual:first");
    }

    fn on_last_public_release(&self) {
        self.log.push("dual:last");
    }
}

impl Drop for Dual {
    fn drop(&mut self) {
        self.log.push("dual:drop");
    }
}

impl IThingImpl for Dual {
    fn Value(&self) -> u32 {
        2
    }
}

/// A container that owns two children.
#[implement(IThing)]
struct Container {
    /// The events of the object.
    log: Arc<Log>,
    /// The children. The container makes them after its own allocation, because a child
    /// needs the interface pointer of the container.
    children: OnceLock<Vec<OwnedObject<Child>>>,
}

impl RefCounted for Container {
    type Policy = DualRefCount;

    fn on_first_public_ref(&self) {
        self.log.push("container:first");
    }

    fn on_last_public_release(&self) {
        self.log.push("container:last");
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        self.log.push("container:drop");
    }
}

impl IThingImpl for Container {
    fn Value(&self) -> u32 {
        3
    }
}

/// A child object. `AddRef` and `Release` go to the container.
#[implement(IChild)]
struct Child {
    /// The index inside the container.
    index: u32,
    /// The interface pointer of the container. The child does not own a reference.
    container: AtomicPtr<c_void>,
    /// The events of the object.
    log: Arc<Log>,
}

impl RefCounted for Child {
    type Policy = ForwardRefCount;

    fn container(&self) -> Option<ptr::NonNull<c_void>> {
        ptr::NonNull::new(self.container.load(Ordering::Relaxed))
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        self.log.push(&format!("child{}:drop", self.index));
    }
}

impl IChildImpl for Child {
    fn Index(&self) -> u32 {
        self.index
    }
}

/// Call `Release` the way a C caller does. The answer is the new public count.
unsafe fn raw_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a valid COM interface pointer and the caller owns one public
    // reference.
    unsafe {
        let vtable = *this.cast::<*const IUnknownVtbl>();
        ((*vtable).Release)(this)
    }
}

/// Call `AddRef` the way a C caller does. The answer is the new public count.
unsafe fn raw_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a valid COM interface pointer and the caller owns a reference.
    unsafe {
        let vtable = *this.cast::<*const IUnknownVtbl>();
        ((*vtable).AddRef)(this)
    }
}

#[test]
fn a_single_count_object_lives_from_the_first_reference_to_the_last() {
    let log = Arc::new(Log::default());
    let object = ComObject::new(Single {
        log: Arc::clone(&log),
    });
    assert_eq!(log.take(), ["single:first"]);
    // SAFETY: The object is alive.
    assert_eq!(unsafe { object.Value() }, 1);

    let this = object.as_raw();
    // SAFETY: The test owns one public reference.
    assert_eq!(unsafe { raw_add_ref(this) }, 2);
    // SAFETY: The test owns two public references.
    assert_eq!(unsafe { raw_release(this) }, 1);
    assert!(log.take().is_empty());

    drop(object);
    assert_eq!(log.take(), ["single:last", "single:drop"]);
}

#[test]
fn a_private_reference_keeps_a_dual_count_object_alive() {
    let log = Arc::new(Log::default());
    let object = ComObject::new(Dual {
        log: Arc::clone(&log),
    });
    assert_eq!(log.take(), ["dual:first"]);

    let private = PrivateRef::<Dual>::from_com_ptr(&object).unwrap();
    assert_eq!(private.public_count(), 1);
    assert_eq!(private.private_count(), 1);

    // The application releases its reference. The object stays alive.
    drop(object);
    assert_eq!(log.take(), ["dual:last"]);
    assert_eq!(private.public_count(), 0);
    assert_eq!(private.private_count(), 1);
    assert_eq!(private.get().Value(), 2);

    // The private reference goes away. The object is destroyed.
    drop(private);
    assert_eq!(log.take(), ["dual:drop"]);
}

#[test]
fn to_public_revives_the_public_count_and_fires_the_hook_again() {
    let log = Arc::new(Log::default());
    let object = ComObject::new(Dual {
        log: Arc::clone(&log),
    });
    let address = object.as_raw();
    let private = PrivateRef::<Dual>::from_com_ptr(&object).unwrap();
    drop(object);
    assert_eq!(log.take(), ["dual:first", "dual:last"]);

    // `GetTexture` gives a public reference of an object that only a private reference
    // holds. The address of the object does not change.
    let again: ComPtr<IThing> = private.to_public();
    assert_eq!(again.as_raw(), address);
    assert_eq!(log.take(), ["dual:first"]);
    assert_eq!(private.public_count(), 1);

    // A second public reference does not fire the hook again.
    let copy = again.clone();
    assert_eq!(private.public_count(), 2);
    assert!(log.take().is_empty());

    drop(copy);
    assert!(log.take().is_empty());
    drop(again);
    assert_eq!(log.take(), ["dual:last"]);
    drop(private);
    assert_eq!(log.take(), ["dual:drop"]);
}

#[test]
fn release_gives_the_new_public_count() {
    let log = Arc::new(Log::default());
    let object = ComObject::new(Dual {
        log: Arc::clone(&log),
    });
    let private = PrivateRef::<Dual>::from_com_ptr(&object).unwrap();
    let this = object.as_raw();

    // SAFETY: The test owns one public reference.
    assert_eq!(unsafe { raw_add_ref(this) }, 2);
    // SAFETY: The test owns two public references.
    assert_eq!(unsafe { raw_add_ref(this) }, 3);
    // SAFETY: Each call removes one of the references of the test.
    assert_eq!(unsafe { raw_release(this) }, 2);
    // SAFETY: See above.
    assert_eq!(unsafe { raw_release(this) }, 1);
    // The `ComPtr` still owns the last public reference. Give it up first.
    let this = object.into_raw();
    // SAFETY: The test owns the last public reference.
    assert_eq!(unsafe { raw_release(this) }, 0);
    assert_eq!(log.take(), ["dual:first", "dual:last"]);

    // An application that calls `Release` in a loop until the answer is zero stops
    // here. The object is alive because of the private reference.
    assert_eq!(private.public_count(), 0);
    drop(private);
    assert_eq!(log.take(), ["dual:drop"]);
}

#[test]
fn a_private_reference_comes_from_a_raw_pointer_of_the_application() {
    let log = Arc::new(Log::default());
    let object = ComObject::new(Dual {
        log: Arc::clone(&log),
    });
    // `SetTexture(stage, texture)` gets a raw interface pointer and must find the
    // object behind it.
    let raw = object.as_raw();
    // SAFETY: The pointer is a valid interface pointer of a live object.
    let private = unsafe { PrivateRef::<Dual>::from_raw(raw) }.unwrap();
    assert_eq!(private.private_count(), 1);
    assert!(ptr::eq(private.get(), object.as_impl::<Dual>().unwrap()));

    // A pointer that this process did not make gives `None`.
    let mut foreign_object: *const c_void = ptr::from_ref(&FOREIGN_VTABLE).cast();
    let foreign: *mut c_void = ptr::from_mut(&mut foreign_object).cast();
    // SAFETY: The place holds a pointer whose first field is a vtable pointer.
    assert!(unsafe { PrivateRef::<Dual>::from_raw(foreign) }.is_none());
    // SAFETY: A null pointer is a valid argument.
    assert!(unsafe { PrivateRef::<Dual>::from_raw(ptr::null_mut()) }.is_none());
    assert!(object.as_impl::<Single>().is_none());

    drop(object);
    drop(private);
    assert_eq!(log.take(), ["dual:first", "dual:last", "dual:drop"]);
}

/// A vtable that this process did not make.
static FOREIGN_VTABLE: [usize; 8] = [0; 8];

#[test]
fn a_child_sends_its_counts_to_the_container_and_dies_with_it() {
    let log = Arc::new(Log::default());
    let container = ComObject::new(Container {
        log: Arc::clone(&log),
        children: OnceLock::new(),
    });
    let value = container.as_impl::<Container>().unwrap();
    let this = container.as_raw();
    let children: Vec<OwnedObject<Child>> = (0..2)
        .map(|index| {
            OwnedObject::new(Child {
                index,
                container: AtomicPtr::new(this),
                log: Arc::clone(&log),
            })
        })
        .collect();
    assert!(value.children.set(children).is_ok());
    assert_eq!(log.take(), ["container:first"]);
    assert_eq!(container.public_count_of::<Container>(), Some(1));

    // `GetSurfaceLevel` gives a public reference of the child. The count of the
    // container goes up.
    let list = value.children.get().unwrap();
    let child: ComPtr<IChild> = list[1].to_public();
    assert_eq!(container.public_count_of::<Container>(), Some(2));
    // SAFETY: The object is alive.
    assert_eq!(unsafe { child.Index() }, 1);
    assert_ne!(child.as_raw(), this);

    // `AddRef` of the child answers with the count of the container.
    // SAFETY: The test owns a public reference of the child.
    assert_eq!(unsafe { raw_add_ref(child.as_raw()) }, 3);
    // SAFETY: The test owns the reference of the line above.
    assert_eq!(unsafe { raw_release(child.as_raw()) }, 2);

    // `QueryInterface` on the child gives the identity of the child, not of the
    // container.
    let unknown: ComPtr<cppvtable::IUnknown> = child.cast().unwrap();
    assert_eq!(unknown.as_raw(), child.as_raw());
    assert_eq!(container.public_count_of::<Container>(), Some(3));
    drop(unknown);

    drop(child);
    assert_eq!(container.public_count_of::<Container>(), Some(1));
    assert!(log.take().is_empty());

    // The container goes away and destroys the children with itself.
    drop(container);
    assert_eq!(
        log.take(),
        [
            "container:last",
            "container:drop",
            "child0:drop",
            "child1:drop"
        ]
    );
}

#[test]
fn a_child_stays_alive_while_the_application_holds_a_reference_of_it() {
    let log = Arc::new(Log::default());
    let container = ComObject::new(Container {
        log: Arc::clone(&log),
        children: OnceLock::new(),
    });
    let value = container.as_impl::<Container>().unwrap();
    let this = container.as_raw();
    assert!(
        value
            .children
            .set(vec![OwnedObject::new(Child {
                index: 0,
                container: AtomicPtr::new(this),
                log: Arc::clone(&log),
            })])
            .is_ok()
    );
    let child: ComPtr<IChild> = value.children.get().unwrap()[0].to_public();

    // The application releases the container. The child still holds a reference of it,
    // so nothing is destroyed.
    drop(container);
    assert_eq!(log.take(), ["container:first"]);
    // SAFETY: The object is alive.
    assert_eq!(unsafe { child.Index() }, 0);

    // The last reference of the child is the last reference of the container.
    drop(child);
    assert_eq!(
        log.take(),
        ["container:last", "container:drop", "child0:drop"]
    );
}

#[test]
fn the_interface_pointer_of_a_child_is_not_the_pointer_of_the_container() {
    let log = Arc::new(Log::default());
    let container = ComObject::new(Container {
        log: Arc::clone(&log),
        children: OnceLock::new(),
    });
    let value = container.as_impl::<Container>().unwrap();
    let this = container.as_raw();
    assert!(
        value
            .children
            .set(vec![OwnedObject::new(Child {
                index: 5,
                container: AtomicPtr::new(this),
                log: Arc::clone(&log),
            })])
            .is_ok()
    );
    let owned = &value.children.get().unwrap()[0];
    let raw = owned.as_raw::<IChild>();
    assert_ne!(raw, this);

    // The pointer of `as_raw` does not change a count.
    assert_eq!(container.public_count_of::<Container>(), Some(1));
    // SAFETY: `owned` holds a live object.
    let same = unsafe { interface_of::<Child, IChild>(owned.get()) };
    assert_eq!(same, raw);
    assert_eq!(IChild::NAME, "IChild");
}
