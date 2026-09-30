//! Reference count policies.
//!
//! A COM object of this crate has one policy. The policy owns the counts and decides
//! when the object is destroyed. A type selects its policy with
//! [`RefCounted::Policy`]. A type with the default hooks selects [`SingleRefCount`] or
//! [`DualRefCount`] with `#[implement(IThing, refcount = single)]` or
//! `#[implement(IThing, refcount = dual)]`; the macro then writes the `RefCounted`
//! implementation.
//!
//! | Policy | Counts | Destruction |
//! | ------ | ------ | ----------- |
//! | [`SingleRefCount`] | one public count | at public count 0 |
//! | [`DualRefCount`] | one public count and one private count | when both counts are 0 |
//! | [`ForwardRefCount`] | none | with the container |
//!
//! # The rules
//!
//! 1. **`AddRef` needs a reference.** A caller of `AddRef` must already own a public
//!    reference or a [`crate::PrivateRef`]. This is the rule of COM. The crate depends
//!    on it: a thread that has no reference can see an object that another thread
//!    destroys.
//! 2. **`Release` returns the new public count.** An application that calls `Release`
//!    in a loop until the answer is zero works correctly.
//! 3. **The hooks fire on the edges.** [`RefCounted::on_first_public_ref`] fires when
//!    the public count goes from 0 to 1. [`RefCounted::on_last_public_release`] fires
//!    when it goes from 1 to 0. [`crate::ComObject::new`] makes the first transition, so
//!    `on_first_public_ref` fires one time for each new object.
//! 4. **The hooks do not overlap.** The crate holds a lock over the 0 <-> 1 transition.
//!    The two hooks of one object always alternate, also when more than one thread calls
//!    `AddRef` and `Release`.
//! 5. **A hook must not change the public count of its own object.** Such a call waits
//!    for a lock that the same thread holds. A hook may change the counts of other
//!    objects, and it may add or remove a private reference of its own object.
//! 6. **The object stays alive during `on_last_public_release`.** With
//!    [`DualRefCount`] the public count holds one private reference while it is larger
//!    than zero. The crate removes that private reference after the hook. The object is
//!    therefore alive while the hook runs, and a hook that makes a new
//!    [`crate::PrivateRef`] of the object stops the destruction.
//! 7. **Resurrection is permitted.** [`crate::PrivateRef::to_public`] adds a public
//!    reference. When the public count was 0, `on_first_public_ref` fires again. The
//!    address of the object does not change.
//! 8. **Destruction happens one time.** The object model has one destruction site, in
//!    [`crate::ComObject`]. The `Drop` implementation of the Rust type runs there.
//! 9. **A child with [`ForwardRefCount`] dies with its container.** The container owns
//!    a [`crate::ChildObject`] of the child. The child is destroyed when the `Drop` of
//!    the container type drops that handle.
//! 10. **Count operations synchronize lifetime transitions.** Each read-modify-write
//!     uses `AcqRel` and each load uses `Acquire`. The final reference release observes
//!     writes published through earlier releases before destruction. An acquire load
//!     that observes the transition lock being released also observes the completed
//!     hook's writes. These orders provide visibility between threads; atomic loads
//!     with `Relaxed` ordering do not provide that synchronization by themselves.
//!
//! # A Direct3D 9 example
//!
//! A device uses [`DualRefCount`]. A texture uses [`DualRefCount`]. A surface of the
//! texture uses [`ForwardRefCount`].
//!
//! - The texture holds a public reference of the device while the public count of the
//!   texture is larger than zero. `on_first_public_ref` adds it and
//!   `on_last_public_release` removes it. A live resource therefore keeps the device
//!   alive.
//! - The device holds a [`crate::PrivateRef`] of each bound texture. A private
//!   reference never keeps the device alive, so no cycle exists.
//! - `on_last_public_release` of the device clears the state. This drops the private
//!   references. A texture that the application already released is then destroyed.

mod dual;
mod forward;
mod public;
mod single;

use core::ffi::c_void;
use core::ptr::NonNull;

pub use dual::{DualRefCount, DualState};
pub use forward::{ForwardRefCount, ForwardState};
pub use single::{SingleRefCount, SingleState};

use crate::GUID;
use crate::object::{ComImplement, ComObject};

/// The counts of one object and the operations on them.
///
/// The crate supplies [`SingleRefCount`], [`DualRefCount`], and [`ForwardRefCount`].
///
/// # Safety
///
/// An implementation must destroy the object exactly one time, and only after the last
/// reference is gone. Use [`crate::ComObject::destroy`] for the destruction.
/// Invoke lifecycle hooks only on the data of a live `ComObject`, during the public
/// count transition described by the hook, and retain the allocation until it returns.
pub unsafe trait RefCountPolicy: Sized + 'static {
    /// The counts that [`ComObject`] holds for this policy.
    type State: Send + Sync + 'static;

    /// Make the counts of a new object. The public count starts at zero.
    fn new_state() -> Self::State;

    /// Add one public reference. The method returns the new public count.
    ///
    /// # Safety
    ///
    /// `object` must refer to a live object, and the caller must already own a
    /// reference of it.
    unsafe fn add_ref<T>(object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>;

    /// Remove one public reference. The method returns the new public count.
    ///
    /// The object can be destroyed during the call. Do not use `object` after it.
    ///
    /// # Safety
    ///
    /// `object` must refer to a live object, and the caller must own the public
    /// reference that it removes.
    unsafe fn release<T>(object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>;

    /// Give the current public count. Use it for tests and for log messages.
    ///
    /// # Safety
    ///
    /// `object` must refer to a live object.
    unsafe fn public_count<T>(object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>;
}

/// A policy that has a private count.
///
/// [`crate::PrivateRef`] works only with such a policy.
///
/// # Safety
///
/// See [`RefCountPolicy`].
pub unsafe trait PrivatePolicy: RefCountPolicy {
    /// Add one private reference.
    ///
    /// # Safety
    ///
    /// `object` must refer to a live object, and the caller must already own a
    /// reference of it.
    unsafe fn add_private<T>(object: *const ComObject<T>)
    where
        T: ComImplement + RefCounted<Policy = Self>;

    /// Remove one private reference.
    ///
    /// The object can be destroyed during the call. Do not use `object` after it.
    ///
    /// # Safety
    ///
    /// `object` must refer to a live object, and the caller must own the private
    /// reference that it removes.
    unsafe fn release_private<T>(object: *const ComObject<T>)
    where
        T: ComImplement + RefCounted<Policy = Self>;

    /// Give the current private count. Use it for tests and for log messages.
    ///
    /// # Safety
    ///
    /// `object` must refer to a live object.
    unsafe fn private_count<T>(object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>;
}

/// A policy of an object that owns itself.
///
/// [`ComObject::new`] works only with such a policy. An object with
/// [`ForwardRefCount`] belongs to a container, so it uses [`crate::ChildObject`].
///
/// # Safety
///
/// See [`RefCountPolicy`].
pub unsafe trait StandalonePolicy: RefCountPolicy {}

/// The reference count behaviour of an implementation type.
///
/// Each type that `#[implement]` uses must implement this trait. With the default
/// hooks, `#[implement(IThing, refcount = single)]` or `refcount = dual` writes the
/// implementation. It fails to compile when an implemented interface is an
/// [`crate::AgileInterface`] and the type is not `Send + Sync`. Write the
/// implementation by hand for [`ForwardRefCount`] or for custom hooks:
///
/// ```ignore
/// unsafe impl RefCounted for VertexBuffer {
///     type Policy = DualRefCount;
///
///     unsafe fn on_first_public_ref(&self) {
///         self.device.add_public_ref();
///     }
///
///     unsafe fn on_last_public_release(&self) {
///         self.device.release_public_ref();
///     }
/// }
/// ```
/// # Safety
///
/// `container` and `query_extra` must return valid live COM interface pointers. A
/// `query_extra` pointer must own one public reference. With `ForwardRefCount`, the
/// container must own the child handle and retain it until the container is destroyed;
/// every public child reference must keep that container alive. The hooks must not
/// change the public count of their own object. If any implemented interface is an
/// [`crate::AgileInterface`], `Self` must implement `Send + Sync` and its hooks must
/// support concurrent calls and destruction on arbitrary threads.
pub unsafe trait RefCounted: Sized + 'static {
    /// The reference count policy of the type.
    type Policy: RefCountPolicy;

    /// The public count went from 0 to 1.
    ///
    /// The hook must not add or remove a public reference of its own object. See the
    /// rules of this module.
    ///
    /// # Safety
    ///
    /// `self` must be the data of a live `ComObject` whose public count is making the
    /// transition from zero to one. The allocation must stay alive during the hook.
    unsafe fn on_first_public_ref(&self) {}

    /// The public count went from 1 to 0.
    ///
    /// The object is still alive. The hook may release the references that it holds of
    /// other objects. The hook must not add or remove a public reference of its own
    /// object.
    ///
    /// # Safety
    ///
    /// `self` must be the data of a live `ComObject` whose public count is making the
    /// transition from one to zero. The allocation must stay alive during the hook.
    unsafe fn on_last_public_release(&self) {}

    /// Give an interface pointer of the container.
    ///
    /// [`ForwardRefCount`] sends `AddRef` and `Release` to this pointer. A type with
    /// another policy does not use the hook. With `ForwardRefCount` this must return
    /// a valid container; the default is only for policies that do not forward.
    ///
    /// # Safety
    ///
    /// `self` must be the data of a live `ComObject`. The caller must keep the object
    /// and its container alive while using the returned pointer.
    unsafe fn container(&self) -> Option<NonNull<c_void>> {
        None
    }

    /// Answer `QueryInterface` for an interface that the object does not implement.
    ///
    /// The generated `QueryInterface` calls this hook after it looks in its own table.
    /// The hook must add the reference of the pointer that it returns, and the pointer
    /// must support the interface identified by `iid` with its required vtable layout.
    ///
    /// # Safety
    ///
    /// `self` must be the data of a live `ComObject` kept alive throughout the call.
    unsafe fn query_extra(&self, iid: &GUID) -> Option<NonNull<c_void>> {
        let _ = iid;
        None
    }
}
