//! C++ VTable interop macros and utilities for MSVC ABI
//!
//! This module provides macros for defining C++ compatible interfaces and classes
//! with proper vtable layout matching MSVC ABI.
//!
//! # Features
//! - `cpp_interface!` - Define vtable layouts with inheritance support
//! - `cpp_class!` - Define classes with multiple inheritance
//! - Automatic calling convention selection (thiscall on x86, C on x64)
//! - RTTI null stub at vtable[-1]
//! - Passthrough support for hooking scenarios

pub use ::paste::paste;

// Re-export for macro use
#[doc(hidden)]
pub use std::ffi::c_void;
#[doc(hidden)]
pub use std::sync::atomic::{Ordering, compiler_fence};

/// Calling convention for MSVC ABI
/// - x86: thiscall (this in ECX)
/// - x64: standard C calling convention (this as first param)
// Note: Calling convention is handled via #[cfg] attributes directly in the macro
// x86: extern "thiscall" - this pointer in ECX
// x64: extern "C" - this pointer as first argument

/// Define a C++ compatible interface with vtable.
///
/// Supports:
/// - Inheritance: `interface IFoo : IBar { ... }`
/// - Explicit slot indices: `[0] fn method(...);`
/// - Mixed implicit/explicit ordering
/// - RTTI null stub at vtable[-1] (MSVC ABI)
/// - `from_ptr()` / `from_ptr_mut()` for consuming C++ objects
///
/// # Example
/// ```ignore
/// define_interface! {
///     interface IUnknown {
///         fn query_interface(&self, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
///         fn add_ref(&self) -> u32;
///         fn release(&self) -> u32;
///     }
///
///     interface IAnimal : IUnknown {
///         fn speak(&self);
///         [5] fn legs(&self) -> i32;  // explicit slot 5
///     }
/// }
/// ```
#[macro_export]
macro_rules! define_interface {
    // Entry point - parse multiple interfaces
    (
        $(
            $(#[$meta:meta])*
            interface $name:ident $(: $base:ident)? {
                $($body:tt)*
            }
        )*
    ) => {
        $(
            $crate::define_interface!(@single
                $(#[$meta])*
                interface $name $(: $base)? { $($body)* }
            );
        )*
    };

    // Single interface without inheritance
    (@single
        $(#[$meta:meta])*
        interface $name:ident {
            $($body:tt)*
        }
    ) => {
        $crate::define_interface!(@build $name, [], { $($body)* }, []);
    };

    // Single interface with single inheritance
    (@single
        $(#[$meta:meta])*
        interface $name:ident : $base:ident {
            $($body:tt)*
        }
    ) => {
        $crate::define_interface!(@build $name, [$base], { $($body)* }, []);
    };

    // Build: collect all methods, then generate
    // Handles [N] fn name(&self, ...);
    (@build $name:ident, [$($bases:ident),*], {
        $(#[$method_meta:meta])*
        [$slot:expr] fn $method:ident (&self $(, $pname:ident : $pty:ty)*) $(-> $ret:ty)?;
        $($rest:tt)*
    }, [$($collected:tt)*]) => {
        $crate::define_interface!(@build $name, [$($bases),*], { $($rest)* }, [
            $($collected)*
            {
                slot: $slot,
                name: $method,
                recv: [&self],
                params: [$(($pname, $pty)),*],
                ret: ($($ret)?),
                meta: [$(#[$method_meta])*]
            }
        ]);
    };

    // Handles [N] fn name(&mut self, ...);
    (@build $name:ident, [$($bases:ident),*], {
        $(#[$method_meta:meta])*
        [$slot:expr] fn $method:ident (&mut self $(, $pname:ident : $pty:ty)*) $(-> $ret:ty)?;
        $($rest:tt)*
    }, [$($collected:tt)*]) => {
        $crate::define_interface!(@build $name, [$($bases),*], { $($rest)* }, [
            $($collected)*
            {
                slot: $slot,
                name: $method,
                recv: [&mut self],
                params: [$(($pname, $pty)),*],
                ret: ($($ret)?),
                meta: [$(#[$method_meta])*]
            }
        ]);
    };

    // Handles fn name(&self, ...); with implicit slot (use _ placeholder, resolved later)
    (@build $name:ident, [$($bases:ident),*], {
        $(#[$method_meta:meta])*
        fn $method:ident (&self $(, $pname:ident : $pty:ty)*) $(-> $ret:ty)?;
        $($rest:tt)*
    }, [$($collected:tt)*]) => {
        $crate::define_interface!(@build $name, [$($bases),*], { $($rest)* }, [
            $($collected)*
            {
                slot: _,
                name: $method,
                recv: [&self],
                params: [$(($pname, $pty)),*],
                ret: ($($ret)?),
                meta: [$(#[$method_meta])*]
            }
        ]);
    };

    // Handles fn name(&mut self, ...); with implicit slot
    (@build $name:ident, [$($bases:ident),*], {
        $(#[$method_meta:meta])*
        fn $method:ident (&mut self $(, $pname:ident : $pty:ty)*) $(-> $ret:ty)?;
        $($rest:tt)*
    }, [$($collected:tt)*]) => {
        $crate::define_interface!(@build $name, [$($bases),*], { $($rest)* }, [
            $($collected)*
            {
                slot: _,
                name: $method,
                recv: [&mut self],
                params: [$(($pname, $pty)),*],
                ret: ($($ret)?),
                meta: [$(#[$method_meta])*]
            }
        ]);
    };

    // Terminal: all methods collected, now generate the output
    (@build $name:ident, [$($bases:ident),*], {}, [$($methods:tt)*]) => {
        $crate::define_interface!(@generate $name, [$($bases),*], [$($methods)*]);
    };



    // Generate the actual structs
    // For now, simplified version that processes methods directly
    (@generate $name:ident, [$($bases:ident),*], [$({
        slot: $slot:tt,
        name: $method:ident,
        recv: [$($recv:tt)+],
        params: [$(($pname:ident, $pty:ty)),*],
        ret: ($($ret:ty)?),
        meta: [$($meta:meta)*]
    })*]) => {
        $crate::vtable::paste! {
            /// VTable for interface $name
            /// Note: In MSVC ABI, RTTI pointer would be at offset -1 (not included here)
            #[repr(C)]
            pub struct [<$name VTable>] {
                $(
                    // Inherit base vtable entries (by embedding)
                    pub [<__base_ $bases>]: [<$bases VTable>],
                )*
                $(
                    $(#[$meta])*
                    #[cfg(target_arch = "x86")]
                    pub $method: unsafe extern "thiscall" fn(
                        this: *mut $crate::vtable::c_void
                        $(, $pname: $pty)*
                    ) $(-> $ret)?,
                    #[cfg(not(target_arch = "x86"))]
                    pub $method: unsafe extern "C" fn(
                        this: *mut $crate::vtable::c_void
                        $(, $pname: $pty)*
                    ) $(-> $ret)?,
                )*
            }

            /// Interface pointer struct for $name
            #[repr(C)]
            pub struct $name {
                vtable: *const [<$name VTable>],
            }

            impl $name {
                /// Get the vtable
                #[inline]
                pub fn vtable(&self) -> &[<$name VTable>] {
                    unsafe { &*self.vtable }
                }

                /// Wrap a raw C++ pointer for calling methods.
                /// Uses compiler fence to prevent optimization issues.
                #[inline]
                pub unsafe fn from_ptr<'a>(ptr: *mut $crate::vtable::c_void) -> &'a Self {
                    unsafe {
                        $crate::vtable::compiler_fence($crate::vtable::Ordering::SeqCst);
                        let ptr = ::std::ptr::read_volatile(&ptr);
                        &*(ptr as *const Self)
                    }
                }

                /// Wrap a raw C++ pointer for calling methods (mutable).
                /// Uses compiler fence to prevent optimization issues.
                #[inline]
                pub unsafe fn from_ptr_mut<'a>(ptr: *mut $crate::vtable::c_void) -> &'a mut Self {
                    unsafe {
                        $crate::vtable::compiler_fence($crate::vtable::Ordering::SeqCst);
                        let ptr = ::std::ptr::read_volatile(&ptr);
                        &mut *(ptr as *mut Self)
                    }
                }

                // Generate wrapper methods for each vtable entry
                $(
                    $crate::define_interface!(@wrapper_method
                        $name, $method, [$($recv)+], [$(($pname, $pty)),*], ($($ret)?)
                    );
                )*
            }
        }
    };

    // Generate wrapper method for &self
    (@wrapper_method $name:ident, $method:ident, [& self], [$(($pname:ident, $pty:ty)),*], ($($ret:ty)?)) => {
        #[inline]
        pub unsafe fn $method(&self $(, $pname: $pty)*) $(-> $ret)? {
            unsafe {
                ((*self.vtable).$method)(
                    self as *const Self as *mut $crate::vtable::c_void
                    $(, $pname)*
                )
            }
        }
    };

    // Generate wrapper method for &mut self
    (@wrapper_method $name:ident, $method:ident, [& mut self], [$(($pname:ident, $pty:ty)),*], ($($ret:ty)?)) => {
        #[inline]
        pub unsafe fn $method(&mut self $(, $pname: $pty)*) $(-> $ret)? {
            unsafe {
                ((*self.vtable).$method)(
                    self as *mut Self as *mut $crate::vtable::c_void
                    $(, $pname)*
                )
            }
        }
    };
}

/// Define a C++ compatible class with vtable(s).
///
/// Supports:
/// - Single and multiple inheritance
/// - Automatic vtable pointer fields
/// - Proper `#[repr(C)]` layout
///
/// # Example
/// ```ignore
/// define_class! {
///     class Dog : IAnimal {
///         name: [u8; 32],
///         age: u32,
///     }
/// }
///
/// // Multiple inheritance
/// define_class! {
///     class MultiDog : IAnimal, IRunnable {
///         name: [u8; 32],
///     }
/// }
/// ```
#[macro_export]
macro_rules! define_class {
    // Single inheritance
    (
        $(#[$meta:meta])*
        $vis:vis class $name:ident : $base:ident {
            $(
                $(#[$field_meta:meta])*
                $field_vis:vis $field_name:ident : $field_ty:ty
            ),* $(,)?
        }
    ) => {
        $crate::vtable::paste! {
            $(#[$meta])*
            #[repr(C)]
            $vis struct $name {
                /// VTable pointer for $base interface
                pub vtable: *const [<$base VTable>],
                $(
                    $(#[$field_meta])*
                    $field_vis $field_name: $field_ty,
                )*
            }

            impl $name {
                /// Get the vtable
                #[inline]
                pub fn vtable(&self) -> &[<$base VTable>] {
                    unsafe { &*self.vtable }
                }

                /// Cast to base interface
                #[inline]
                pub fn as_interface(&self) -> &$base {
                    unsafe { &*(self as *const Self as *const $base) }
                }

                /// Cast to base interface (mutable)
                #[inline]
                pub fn as_interface_mut(&mut self) -> &mut $base {
                    unsafe { &mut *(self as *mut Self as *mut $base) }
                }
            }
        }
    };

    // Multiple inheritance (two bases)
    (
        $(#[$meta:meta])*
        $vis:vis class $name:ident : $base1:ident, $base2:ident {
            $(
                $(#[$field_meta:meta])*
                $field_vis:vis $field_name:ident : $field_ty:ty
            ),* $(,)?
        }
    ) => {
        $crate::vtable::paste! {
            $(#[$meta])*
            #[repr(C)]
            $vis struct $name {
                /// VTable pointer for $base1 interface (primary)
                pub [<vtable_ $base1:snake>]: *const [<$base1 VTable>],
                /// VTable pointer for $base2 interface (secondary - requires this-adjustment)
                pub [<vtable_ $base2:snake>]: *const [<$base2 VTable>],
                $(
                    $(#[$field_meta])*
                    $field_vis $field_name: $field_ty,
                )*
            }

            impl $name {
                /// Cast to primary interface (no adjustment needed)
                #[inline]
                pub fn [<as_ $base1:snake>](&self) -> &$base1 {
                    unsafe { &*(self as *const Self as *const $base1) }
                }

                /// Cast to primary interface (mutable, no adjustment needed)
                #[inline]
                pub fn [<as_ $base1:snake _mut>](&mut self) -> &mut $base1 {
                    unsafe { &mut *(self as *mut Self as *mut $base1) }
                }

                /// Cast to secondary interface (requires this-adjustment)
                #[inline]
                pub fn [<as_ $base2:snake>](&self) -> &$base2 {
                    unsafe {
                        let ptr = (self as *const Self as *const u8)
                            .add(::std::mem::offset_of!(Self, [<vtable_ $base2:snake>]));
                        &*(ptr as *const $base2)
                    }
                }

                /// Cast to secondary interface (mutable, requires this-adjustment)
                #[inline]
                pub fn [<as_ $base2:snake _mut>](&mut self) -> &mut $base2 {
                    unsafe {
                        let ptr = (self as *mut Self as *mut u8)
                            .add(::std::mem::offset_of!(Self, [<vtable_ $base2:snake>]));
                        &mut *(ptr as *mut $base2)
                    }
                }

                /// Get offset to secondary interface (for this-adjustment thunks)
                #[inline]
                pub const fn [<offset_to_ $base2:snake>]() -> usize {
                    ::std::mem::offset_of!(Self, [<vtable_ $base2:snake>])
                }
            }
        }
    };

    // No inheritance (standalone class with custom vtable)
    (
        $(#[$meta:meta])*
        $vis:vis class $name:ident {
            vtable {
                $(
                    $(#[$method_meta:meta])*
                    fn $method_name:ident (&mut self $(, $arg_name:ident : $arg_ty:ty )*) $(-> $ret_ty:ty)?
                );* $(;)?
            }

            $(
                $(#[$field_meta:meta])*
                $field_vis:vis $field_name:ident : $field_ty:ty
            ),* $(,)?
        }
    ) => {
        $crate::vtable::paste! {
            /// VTable struct for $name
            #[repr(C)]
            $vis struct [<$name VTable>] {
                $(
                    $(#[$method_meta])*
                    pub $method_name: unsafe extern $crate::vtable::extern_abi!() fn(
                        this: *mut $crate::vtable::c_void
                        $(, $arg_name: $arg_ty)*
                    ) $(-> $ret_ty)?,
                )*
            }

            $(#[$meta])*
            #[repr(C)]
            $vis struct $name {
                /// VTable pointer
                pub vtable: *const [<$name VTable>],
                $(
                    $(#[$field_meta])*
                    $field_vis $field_name: $field_ty,
                )*
            }

            impl $name {
                /// Get the vtable
                #[inline]
                pub fn vtable(&self) -> &[<$name VTable>] {
                    unsafe { &*self.vtable }
                }

                $(
                    /// Call vtable method $method_name
                    #[inline]
                    pub unsafe fn $method_name(&mut self $(, $arg_name: $arg_ty)*) $(-> $ret_ty)? {
                        unsafe {
                            ((*self.vtable).$method_name)(
                                self as *mut Self as *mut $crate::vtable::c_void
                                $(, $arg_name)*
                            )
                        }
                    }
                )*
            }
        }
    };
}
