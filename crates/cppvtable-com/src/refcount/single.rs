//! The standard COM reference count: one public count.

use core::sync::atomic::{Ordering, fence};

use super::public::PublicCount;
use super::{RefCountPolicy, RefCounted, StandalonePolicy};
use crate::object::{ComImplement, ComObject};

/// The counts of [`SingleRefCount`].
#[derive(Debug)]
pub struct SingleState {
    /// The public count.
    public: PublicCount,
}

/// The standard COM reference count.
///
/// One atomic public count. The object is destroyed when the count goes to zero.
/// `on_last_public_release` runs before the destruction.
///
/// Use this policy for an object that has no private references: a factory object, a
/// state block, a query.
#[derive(Debug)]
pub struct SingleRefCount;

// SAFETY: `release` destroys the object only on the transition from 1 to 0. The lock bit
// of `PublicCount` gives exactly one thread that transition.
unsafe impl RefCountPolicy for SingleRefCount {
    type State = SingleState;

    fn new_state() -> Self::State {
        SingleState {
            public: PublicCount::new(),
        }
    }

    unsafe fn add_ref<T>(object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>,
    {
        // SAFETY: The caller owns a reference, so the object is live.
        let state = unsafe { (*object).state() };
        let edge = state.public.add();
        if edge.crossed {
            // SAFETY: The object is live.
            unsafe { (*object).data().on_first_public_ref() };
            state.public.unlock();
        }
        edge.count
    }

    unsafe fn release<T>(object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>,
    {
        // SAFETY: The caller owns the reference that it removes, so the object is live.
        let state = unsafe { (*object).state() };
        let edge = state.public.sub();
        if edge.crossed {
            // SAFETY: The object is live until the call of `destroy`.
            unsafe { (*object).data().on_last_public_release() };
            state.public.unlock();
            // The fence makes the writes of all other threads visible before the
            // destructor runs. Each `Release` of another thread wrote with `AcqRel`.
            fence(Ordering::Acquire);
            // SAFETY: The count went from 1 to 0. Only this thread makes that
            // transition, so this is the only destruction of the object.
            unsafe { ComObject::destroy(object) };
        }
        edge.count
    }

    unsafe fn public_count<T>(object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>,
    {
        // SAFETY: The caller gives a live object.
        unsafe { (*object).state() }.public.get()
    }
}

// SAFETY: The policy owns the lifetime of the object. `ComObject::new` may use it.
unsafe impl StandalonePolicy for SingleRefCount {}
