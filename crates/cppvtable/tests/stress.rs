//! Many threads change the counts of one object at the same time.
//!
//! The tests check that:
//!
//! - The counts come back to the start value, and the object is destroyed exactly one
//!   time.
//! - The hooks of the 0 <-> 1 transitions always alternate. Two hooks never run at the
//!   same time, and `on_first_public_ref` never runs twice without an
//!   `on_last_public_release` between them.
//! - `to_public` from more than one thread brings an object back from public count zero
//!   without a double destruction.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

use cppvtable::{
    ComObject, ComPtr, DualRefCount, IUnknownVtbl, PrivateRef, RefCounted, SingleRefCount,
    implement, interface,
};

/// The number of threads of each test.
const THREADS: usize = 8;

/// The number of operations of each thread.
const ROUNDS: usize = 2_000;

/// A small interface for the objects of this test.
#[interface(abi = com, iid = "57e55001-0000-4000-8000-000000000001")]
pub unsafe trait IStress {
    /// Give the value of the object.
    fn Value(&self) -> u32;
}

/// The counters of the test.
#[derive(Default)]
struct Counters {
    /// The number of destructions.
    drops: AtomicU32,
    /// The number of `on_first_public_ref` calls.
    first: AtomicU32,
    /// The number of `on_last_public_release` calls.
    last: AtomicU32,
    /// True while a hook runs. Two hooks must never overlap.
    inside: AtomicBool,
    /// True when the object has a public reference. The hooks must alternate.
    public: AtomicBool,
    /// The number of rule failures that the hooks found.
    failures: AtomicU32,
}

impl Counters {
    /// Run the checks of a hook. `enter` is true for `on_first_public_ref`.
    fn hook(&self, enter: bool) {
        if self.inside.swap(true, Ordering::AcqRel) {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
        if self.public.swap(enter, Ordering::AcqRel) == enter {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
        if enter {
            self.first.fetch_add(1, Ordering::Relaxed);
        } else {
            self.last.fetch_add(1, Ordering::Relaxed);
        }
        self.inside.store(false, Ordering::Release);
    }
}

/// An object with the standard COM reference count.
#[implement(IStress)]
struct Single {
    /// The counters of the test.
    counters: Arc<Counters>,
}

impl RefCounted for Single {
    type Policy = SingleRefCount;

    fn on_first_public_ref(&self) {
        self.counters.hook(true);
    }

    fn on_last_public_release(&self) {
        self.counters.hook(false);
    }
}

impl Drop for Single {
    fn drop(&mut self) {
        self.counters.drops.fetch_add(1, Ordering::Relaxed);
    }
}

impl IStressImpl for Single {
    fn Value(&self) -> u32 {
        1
    }
}

/// An object with a public count and a private count.
#[implement(IStress)]
struct Dual {
    /// The counters of the test.
    counters: Arc<Counters>,
}

impl RefCounted for Dual {
    type Policy = DualRefCount;

    fn on_first_public_ref(&self) {
        self.counters.hook(true);
    }

    fn on_last_public_release(&self) {
        self.counters.hook(false);
    }
}

impl Drop for Dual {
    fn drop(&mut self) {
        self.counters.drops.fetch_add(1, Ordering::Relaxed);
    }
}

impl IStressImpl for Dual {
    fn Value(&self) -> u32 {
        2
    }
}

#[test]
fn many_threads_add_and_remove_public_references() {
    let counters = Arc::new(Counters::default());
    let object = ComObject::new(Single {
        counters: Arc::clone(&counters),
    });

    thread::scope(|scope| {
        for _ in 0..THREADS {
            let thread_copy = object.clone();
            scope.spawn(move || {
                for _ in 0..ROUNDS {
                    let extra = thread_copy.clone();
                    // SAFETY: This thread owns a public reference.
                    assert_eq!(unsafe { extra.Value() }, 1);
                    drop(extra);
                }
            });
        }
    });

    // Only the first reference is left.
    assert_eq!(object.public_count_of::<Single>(), Some(1));
    assert_eq!(counters.drops.load(Ordering::Relaxed), 0);
    drop(object);
    assert_eq!(counters.drops.load(Ordering::Relaxed), 1);
    assert_eq!(counters.first.load(Ordering::Relaxed), 1);
    assert_eq!(counters.last.load(Ordering::Relaxed), 1);
    assert_eq!(counters.failures.load(Ordering::Relaxed), 0);
}

#[test]
fn many_threads_bring_an_object_back_from_public_count_zero() {
    let counters = Arc::new(Counters::default());
    let object = ComObject::new(Dual {
        counters: Arc::clone(&counters),
    });
    let private = PrivateRef::<Dual>::from_com_ptr(&object).unwrap();
    // The application releases its reference. Only the private reference is left.
    drop(object);
    assert_eq!(private.public_count(), 0);

    thread::scope(|scope| {
        for _ in 0..THREADS {
            let thread_copy = private.clone();
            scope.spawn(move || {
                for _ in 0..ROUNDS {
                    // `GetTexture` from more than one thread. Each call brings the
                    // public count up, and the last release brings it down again.
                    let public: ComPtr<IStress> = thread_copy.to_public();
                    // SAFETY: This thread owns a public reference.
                    assert_eq!(unsafe { public.Value() }, 2);
                    drop(public);
                }
            });
        }
    });

    assert_eq!(private.public_count(), 0);
    assert_eq!(private.private_count(), 1);
    assert_eq!(counters.drops.load(Ordering::Relaxed), 0);
    assert_eq!(counters.failures.load(Ordering::Relaxed), 0);
    // The hooks always alternate, so the two numbers are the same.
    assert_eq!(
        counters.first.load(Ordering::Relaxed),
        counters.last.load(Ordering::Relaxed)
    );
    assert!(counters.first.load(Ordering::Relaxed) >= 1);

    drop(private);
    assert_eq!(counters.drops.load(Ordering::Relaxed), 1);
}

#[test]
fn many_threads_mix_public_and_private_references() {
    let counters = Arc::new(Counters::default());
    let object = ComObject::new(Dual {
        counters: Arc::clone(&counters),
    });
    let private = PrivateRef::<Dual>::from_com_ptr(&object).unwrap();

    thread::scope(|scope| {
        for index in 0..THREADS {
            let public_copy = object.clone();
            let private_copy = private.clone();
            scope.spawn(move || {
                for round in 0..ROUNDS {
                    if (round + index) % 2 == 0 {
                        drop(public_copy.clone());
                    } else {
                        drop(private_copy.clone());
                    }
                }
            });
        }
    });

    assert_eq!(private.public_count(), 1);
    assert_eq!(private.private_count(), 1);
    assert_eq!(counters.failures.load(Ordering::Relaxed), 0);
    drop(object);
    assert_eq!(counters.drops.load(Ordering::Relaxed), 0);
    drop(private);
    assert_eq!(counters.drops.load(Ordering::Relaxed), 1);
}

#[test]
fn a_c_caller_on_many_threads_sees_a_count_that_never_goes_wrong() {
    let counters = Arc::new(Counters::default());
    let object = ComObject::new(Single {
        counters: Arc::clone(&counters),
    });
    let address = object.as_raw() as usize;

    thread::scope(|scope| {
        for _ in 0..THREADS {
            let keep_alive = object.clone();
            scope.spawn(move || {
                let this = keep_alive.as_raw();
                // SAFETY: `this` is a valid COM interface pointer of a live object.
                let vtable = unsafe { *this.cast::<*const IUnknownVtbl>() };
                for _ in 0..ROUNDS {
                    // SAFETY: This thread owns a public reference, so `AddRef` is
                    // permitted. The answer must be at least 2.
                    let after_add = unsafe { ((*vtable).AddRef)(this) };
                    assert!(after_add >= 2);
                    // SAFETY: This call removes the reference of the line above.
                    let after_release = unsafe { ((*vtable).Release)(this) };
                    assert!(after_release >= 1);
                }
            });
        }
    });

    assert_eq!(object.as_raw() as usize, address);
    assert_eq!(object.public_count_of::<Single>(), Some(1));
    assert_eq!(counters.failures.load(Ordering::Relaxed), 0);
    drop(object);
    assert_eq!(counters.drops.load(Ordering::Relaxed), 1);
}
