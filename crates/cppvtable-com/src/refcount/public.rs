//! The public reference count and the lock of the 0 <-> 1 transition.
//!
//! The public count is the count that `AddRef` and `Release` change and return. A
//! transition from 0 to 1 and a transition from 1 to 0 run a hook of
//! [`crate::RefCounted`]. The hooks must run in the correct order, and two hooks must
//! not run at the same time. The count therefore holds a lock bit.
//!
//! - Bit 31 is the lock bit. Bits 0 to 30 are the count.
//! - A thread that makes a 0 -> 1 or a 1 -> 0 transition sets the lock bit with the same
//!   atomic operation that changes the count. It clears the bit after the hook.
//! - A thread that changes the count between two values that are both larger than zero
//!   does not use the lock. It keeps the lock bit as it is. An `AddRef` during a hook
//!   therefore does not wait.
//! - A thread that needs a transition while the lock bit is set waits in a spin loop.
//!
//! A hook must not add or remove a public reference of its own object. Such a call waits
//! for a lock that the same thread holds.
//!
//! # Memory order
//!
//! Every read-modify-write uses `AcqRel`, and every load uses `Acquire`. The reasons:
//!
//! - A thread that sees the lock bit clear must see all writes of the hook that held the
//!   lock. The `Release` part of `unlock` and the `Acquire` part of the load give that.
//! - A thread that makes the last transition must see all writes of every other thread,
//!   because the destructor runs after it.
//! - An `Acquire` load also stops the optimizer from moving the load out of the spin
//!   loop. A `Relaxed` load in a loop that only waits is not safe against that.
//!
//! A stronger order never makes the count wrong. On x86 a read-modify-write always has
//! the `lock` prefix, so the stronger order costs nothing there.

use core::sync::atomic::{AtomicU32, Ordering};

/// The lock bit of the public count.
const LOCK: u32 = 1 << 31;

/// The mask of the count part of the public count.
const COUNT: u32 = LOCK - 1;

/// The result of a change of the public count.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Edge {
    /// The new public count.
    pub(crate) count: u32,
    /// True when the count went from 0 to 1 or from 1 to 0. The caller holds the lock
    /// and must call [`PublicCount::unlock`] after the hook.
    pub(crate) crossed: bool,
}

/// An atomic public reference count with a lock bit.
#[derive(Debug)]
pub(crate) struct PublicCount(AtomicU32);

impl PublicCount {
    /// Make a count that starts at zero.
    pub(crate) const fn new() -> Self {
        Self(AtomicU32::new(0))
    }

    /// Give the current count.
    pub(crate) fn get(&self) -> u32 {
        self.0.load(Ordering::Acquire) & COUNT
    }

    /// Add one reference.
    pub(crate) fn add(&self) -> Edge {
        loop {
            let current = self.0.load(Ordering::Acquire);
            let count = current & COUNT;
            if count == 0 {
                if current & LOCK != 0 {
                    core::hint::spin_loop();
                    continue;
                }
                if self
                    .0
                    .compare_exchange_weak(0, 1 | LOCK, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Edge {
                        count: 1,
                        crossed: true,
                    };
                }
            } else if count == COUNT {
                // The count is at the maximum. Keep it there. This never happens with a
                // correct application.
                return Edge {
                    count,
                    crossed: false,
                };
            } else if self
                .0
                .compare_exchange_weak(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Edge {
                    count: count + 1,
                    crossed: false,
                };
            }
            core::hint::spin_loop();
        }
    }

    /// Remove one reference.
    pub(crate) fn sub(&self) -> Edge {
        loop {
            let current = self.0.load(Ordering::Acquire);
            let count = current & COUNT;
            if count == 0 {
                // The application released more times than it added. Do nothing.
                return Edge {
                    count: 0,
                    crossed: false,
                };
            }
            if count == 1 {
                if current & LOCK != 0 {
                    core::hint::spin_loop();
                    continue;
                }
                if self
                    .0
                    .compare_exchange_weak(1, LOCK, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Edge {
                        count: 0,
                        crossed: true,
                    };
                }
            } else if self
                .0
                .compare_exchange_weak(current, current - 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Edge {
                    count: count - 1,
                    crossed: false,
                };
            }
            core::hint::spin_loop();
        }
    }

    /// Clear the lock bit after a hook.
    ///
    /// The `Release` part makes the writes of the hook visible to the next thread that
    /// takes the lock.
    pub(crate) fn unlock(&self) {
        self.0.fetch_and(COUNT, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::PublicCount;

    #[test]
    fn transitions_report_the_edges() {
        let count = PublicCount::new();

        let first = count.add();
        assert_eq!(first.count, 1);
        assert!(first.crossed);
        count.unlock();
        assert_eq!(count.get(), 1);

        let second = count.add();
        assert_eq!(second.count, 2);
        assert!(!second.crossed);

        let third = count.sub();
        assert_eq!(third.count, 1);
        assert!(!third.crossed);

        let last = count.sub();
        assert_eq!(last.count, 0);
        assert!(last.crossed);
        count.unlock();
        assert_eq!(count.get(), 0);
    }

    #[test]
    fn a_release_without_a_reference_does_nothing() {
        let count = PublicCount::new();
        let edge = count.sub();
        assert_eq!(edge.count, 0);
        assert!(!edge.crossed);
    }

    #[test]
    fn a_count_that_is_larger_than_one_changes_while_the_lock_is_set() {
        let count = PublicCount::new();
        // Take the lock with the transition from 0 to 1 and keep it.
        assert!(count.add().crossed);
        // A second `AddRef` does not wait for the lock.
        let second = count.add();
        assert_eq!(second.count, 2);
        assert!(!second.crossed);
        // A `Release` back to 1 does not wait either.
        let third = count.sub();
        assert_eq!(third.count, 1);
        assert!(!third.crossed);
        count.unlock();
        assert_eq!(count.get(), 1);
    }
}
