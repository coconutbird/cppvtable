//! The reference count of a child object: `AddRef` and `Release` go to the container.

use super::{RefCountPolicy, RefCounted};
use crate::interface::{unknown_add_ref, unknown_release};
use crate::object::{ComImplement, ComObject};

/// The counts of [`ForwardRefCount`]. The policy has no counts.
#[derive(Debug)]
pub struct ForwardState;

/// `AddRef` and `Release` go to a container object.
///
/// Direct3D 9 uses this rule for a surface of a texture, for a volume of a volume
/// texture, and for the implicit surfaces of a swapchain. `AddRef` of the child changes
/// the public count of the container and returns the new count of the container.
///
/// The child has no counts of its own. The container owns the child with an
/// [`crate::OwnedObject`] handle and destroys it with itself.
///
/// [`RefCounted::container`] gives the interface pointer of the container. Returning
/// `None` violates the policy contract and causes reference-count operations to panic.
#[derive(Debug)]
pub struct ForwardRefCount;

// SAFETY: The policy never destroys the object. The owner of the `OwnedObject` handle
// destroys it exactly one time.
unsafe impl RefCountPolicy for ForwardRefCount {
    type State = ForwardState;

    fn new_state() -> Self::State {
        ForwardState
    }

    unsafe fn add_ref<T>(object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>,
    {
        // SAFETY: The caller owns a reference, so the object is live.
        let container = unsafe { (*object).data().container() };
        match container {
            // SAFETY: `container` gives a valid COM interface pointer of a live object.
            // The container is alive because it owns this child.
            Some(pointer) => unsafe { unknown_add_ref(pointer.as_ptr()) },
            None => panic!("ForwardRefCount requires a live COM container"),
        }
    }

    unsafe fn release<T>(object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>,
    {
        // SAFETY: The caller owns a reference, so the object is live.
        let container = unsafe { (*object).data().container() };
        match container {
            // SAFETY: `container` gives a valid COM interface pointer, and the caller
            // owns the public reference of the container that this call removes.
            Some(pointer) => unsafe { unknown_release(pointer.as_ptr()) },
            None => panic!("ForwardRefCount requires a live COM container"),
        }
    }

    unsafe fn public_count<T>(_object: *const ComObject<T>) -> u32
    where
        T: ComImplement + RefCounted<Policy = Self>,
    {
        // The child has no count. The count of the container is the only count.
        0
    }
}
