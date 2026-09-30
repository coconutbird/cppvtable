//! The Direct3D 9 reference count: one public count and one private count.

use core::sync::atomic::{AtomicU32, Ordering, fence};

use super::public::PublicCount;
use super::{PrivatePolicy, RefCountPolicy, RefCounted, StandalonePolicy};
use crate::object::{ComImplement, ComObject};

/// The counts of [`DualRefCount`].
#[derive(Debug)]
pub struct DualState {
    /// The count that `AddRef` and `Release` change.
    public: PublicCount,
    /// The count of the private references, plus one while the public count is larger
    /// than zero.
    private: AtomicU32,
}

/// A public count and a private count.
///
/// The public count belongs to the application. The private count belongs to the
/// frontend: the device holds a private reference of each bound resource.
///
/// The object is destroyed when both counts are zero. The public count holds one private
/// reference while it is larger than zero, so one atomic operation decides the
/// destruction. The consequences are:
///
/// - The object stays alive during `on_last_public_release`.
/// - A private reference keeps an object alive after the application released it.
///   `GetTexture` then returns the same pointer that the application gave to
///   `SetTexture`.
/// - [`crate::PrivateRef::to_public`] brings the public count back to 1 and fires
///   `on_first_public_ref` again.
#[derive(Debug)]
pub struct DualRefCount;

impl DualRefCount {
    /// Remove one private reference and destroy the object when no reference is left.
    ///
    /// # Safety
    ///
    /// The caller must own the private reference that it removes.
    unsafe fn drop_private<T>(object: *const ComObject<T>)
    where
        T: ComImplement + RefCounted<Policy = Self>,
    {
        // SAFETY: The caller owns a private reference, so the object is live.
        let state = unsafe { (*object).state() };
        if state.private.fetch_sub(1, Ordering::AcqRel) == 1 {
            // The last reference is gone. The fence makes the writes of all other
            // threads visible before the destructor runs. This is the rule of `Arc`:
            // each other thread wrote with `Release`, so one `Acquire` fence here makes
            // all of those writes visible.
            fence(Ordering::Acquire);
            // SAFETY: Exactly one thread sees the old value 1, so this is the only
            // destruction of the object.
            unsafe { ComObject::destroy(object) };
        }
    }
}

// SAFETY: The private count decides the destruction. Exactly one thread sees the
// transition of that count to zero.
unsafe impl RefCountPolicy for DualRefCount {
    type State = DualState;

    fn new_state() -> Self::State {
        DualState {
            public: PublicCount::new(),
            private: AtomicU32::new(0),
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
            // The public count now holds one private reference.
            state.private.fetch_add(1, Ordering::AcqRel);
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
            // The private reference of the public count is still there, so the object
            // is alive during the hook.
            // SAFETY: The object is live.
            unsafe { (*object).data().on_last_public_release() };
            state.public.unlock();
            // SAFETY: This thread owns the private reference of the public count.
            unsafe { Self::drop_private(object) };
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

// SAFETY: `add_private` and `release_private` change only the private count.
// `release_private` destroys the object on the transition of that count to zero.
unsafe impl PrivatePolicy for DualRefCount {
    unsafe fn add_private<T>(object: *const ComObject<T>)
    where
        T: ComImplement + RefCounted<Policy = Self>,
    {
        // SAFETY: The caller owns a reference, so the object is live.
        unsafe { (*object).state() }
            .private
            .fetch_add(1, Ordering::AcqRel);
    }

    unsafe fn release_private<T>(object: *const ComObject<T>)
    where
        T: ComImplement + RefCounted<Policy = Self>,
    {
        // SAFETY: The caller owns the private reference that it removes.
        unsafe { Self::drop_private(object) };
    }

    unsafe fn private_count<T>(object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>,
    {
        // SAFETY: The caller gives a live object.
        let state = unsafe { (*object).state() };
        let all = state.private.load(Ordering::Acquire);
        // Hide the private reference that the public count holds.
        if state.public.get() > 0 {
            all.saturating_sub(1)
        } else {
            all
        }
    }
}

// SAFETY: The policy owns the lifetime of the object. `ComObject::new` may use it.
unsafe impl StandalonePolicy for DualRefCount {}
