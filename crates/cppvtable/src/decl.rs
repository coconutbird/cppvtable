//! Declarative macros for C++ VTable interop (MSVC ABI)
//!
//! These macros provide a more concise syntax that delegates to the proc-macros
//! for actual code generation. This ensures a single implementation while
//! offering a nicer API for common cases.
//!
//! # Features
//! - `define_interface!` - Define vtable layouts (delegates to `#[cpp_interface]`)
//! - `define_class!` - Define classes with vtable pointers and helper methods
//! - Explicit slot indices: `[N] fn method(...);` (becomes `#[slot(N)]`)
//!
//! # Example
//! ```ignore
//! define_interface! {
//!     interface IAnimal {
//!         fn speak(&self);
//!         fn legs(&self) -> i32;
//!     }
//! }
//!
//! define_class! {
//!     class Dog : IAnimal {
//!         name: [u8; 32],
//!     }
//! }
//! ```

/// Define a C++ compatible interface with vtable.
///
/// This macro expands to `#[cpp_interface] pub trait ...` and lets the
/// proc-macro handle all code generation.
///
/// # Syntax
/// ```ignore
/// define_interface! {
///     interface IFoo {
///         fn method(&self);
///         fn method_with_ret(&self) -> i32;
///         fn method_with_args(&self, x: i32, y: f32);
///         [5] fn explicit_slot(&self);  // explicit slot index
///     }
/// }
/// ```
#[macro_export]
macro_rules! define_interface {
    // Entry point - parse multiple interfaces
    (
        $(
            $(#[$meta:meta])*
            interface $name:ident {
                $($body:tt)*
            }
        )*
    ) => {
        $(
            $crate::define_interface!(@single
                $(#[$meta])*
                interface $name { $($body)* }
            );
        )*
    };

    // Single interface - collect methods then emit trait
    // Start with empty collected methods and empty slots accumulator
    (@single
        $(#[$meta:meta])*
        interface $name:ident {
            $($body:tt)*
        }
    ) => {
        $crate::define_interface!(@collect $name, [$(#[$meta])*], { $($body)* }, [], []);
    };

    // Collect: method with explicit slot [N]
    // Stores slot info to pass via cpp_interface attribute argument
    (@collect $name:ident, [$($meta:tt)*], {
        $(#[$method_meta:meta])*
        [$slot:expr] fn $method:ident (&self $(, $pname:ident : $pty:ty)*) $(-> $ret:ty)?;
        $($rest:tt)*
    }, [$($collected:tt)*], [$($slots:tt)*]) => {
        $crate::define_interface!(@collect $name, [$($meta)*], { $($rest)* }, [
            $($collected)*
            { $(#[$method_meta])* fn $method(&self $(, $pname: $pty)*) $(-> $ret)?; }
        ], [$($slots)* $method = $slot,]);
    };

    // Collect: method with explicit slot [N] and &mut self
    (@collect $name:ident, [$($meta:tt)*], {
        $(#[$method_meta:meta])*
        [$slot:expr] fn $method:ident (&mut self $(, $pname:ident : $pty:ty)*) $(-> $ret:ty)?;
        $($rest:tt)*
    }, [$($collected:tt)*], [$($slots:tt)*]) => {
        $crate::define_interface!(@collect $name, [$($meta)*], { $($rest)* }, [
            $($collected)*
            { $(#[$method_meta])* fn $method(&mut self $(, $pname: $pty)*) $(-> $ret)?; }
        ], [$($slots)* $method = $slot,]);
    };

    // Collect: method without explicit slot (&self)
    (@collect $name:ident, [$($meta:tt)*], {
        $(#[$method_meta:meta])*
        fn $method:ident (&self $(, $pname:ident : $pty:ty)*) $(-> $ret:ty)?;
        $($rest:tt)*
    }, [$($collected:tt)*], [$($slots:tt)*]) => {
        $crate::define_interface!(@collect $name, [$($meta)*], { $($rest)* }, [
            $($collected)*
            { $(#[$method_meta])* fn $method(&self $(, $pname: $pty)*) $(-> $ret)?; }
        ], [$($slots)*]);
    };

    // Collect: method without explicit slot (&mut self)
    (@collect $name:ident, [$($meta:tt)*], {
        $(#[$method_meta:meta])*
        fn $method:ident (&mut self $(, $pname:ident : $pty:ty)*) $(-> $ret:ty)?;
        $($rest:tt)*
    }, [$($collected:tt)*], [$($slots:tt)*]) => {
        $crate::define_interface!(@collect $name, [$($meta)*], { $($rest)* }, [
            $($collected)*
            { $(#[$method_meta])* fn $method(&mut self $(, $pname: $pty)*) $(-> $ret)?; }
        ], [$($slots)*]);
    };

    // Terminal: emit the trait with cpp_interface attribute (with slots)
    (@collect $name:ident, [$($meta:tt)*], {}, [$({ $($method:tt)* })*], [$($slots:tt)+]) => {
        $($meta)*
        #[$crate::proc::cpp_interface(slots($($slots)*))]
        pub trait $name {
            $($($method)*)*
        }
    };

    // Terminal: emit the trait with cpp_interface attribute (no slots)
    (@collect $name:ident, [$($meta:tt)*], {}, [$({ $($method:tt)* })*], []) => {
        $($meta)*
        #[$crate::proc::cpp_interface]
        pub trait $name {
            $($($method)*)*
        }
    };
}

/// Define a C++ compatible class with vtable pointer(s).
///
/// This generates the struct with proper vtable fields and helper methods
/// for casting to interfaces. Use `#[implement(Interface)]` separately
/// to provide the method implementations.
///
/// # Single Inheritance
/// ```ignore
/// define_class! {
///     class Dog : IAnimal {
///         name: [u8; 32],
///         age: u32,
///     }
/// }
///
/// #[implement(IAnimal)]
/// impl Dog {
///     fn speak(&self) { println!("Woof!"); }
///     fn legs(&self) -> i32 { 4 }
/// }
/// ```
///
/// # Multiple Inheritance
/// ```ignore
/// define_class! {
///     class Duck : ISwimmer, IFlyer {
///         name: [u8; 16],
///     }
/// }
///
/// #[implement(ISwimmer)]
/// impl Duck {
///     fn swim(&self) { }
/// }
///
/// #[implement(IFlyer)]
/// impl Duck {
///     fn fly(&self) { }
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
        $crate::paste! {
            $(#[$meta])*
            #[repr(C)]
            $vis struct $name {
                /// VTable pointer for $base interface
                pub [<vtable_ $base:snake>]: *const [<$base VTable>],
                $(
                    $(#[$field_meta])*
                    $field_vis $field_name: $field_ty,
                )*
            }

            impl $name {
                /// Cast to interface (no adjustment needed for single inheritance)
                #[inline]
                pub fn [<as_ $base:snake>](&self) -> &$base {
                    unsafe { &*(self as *const Self as *const $base) }
                }

                /// Cast to interface (mutable)
                #[inline]
                pub fn [<as_ $base:snake _mut>](&mut self) -> &mut $base {
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
        $crate::paste! {
            $(#[$meta])*
            #[repr(C)]
            $vis struct $name {
                /// VTable pointer for $base1 interface (primary)
                pub [<vtable_ $base1:snake>]: *const [<$base1 VTable>],
                /// VTable pointer for $base2 interface (secondary)
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

                /// Cast to primary interface (mutable)
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

                /// Cast to secondary interface (mutable)
                #[inline]
                pub fn [<as_ $base2:snake _mut>](&mut self) -> &mut $base2 {
                    unsafe {
                        let ptr = (self as *mut Self as *mut u8)
                            .add(::std::mem::offset_of!(Self, [<vtable_ $base2:snake>]));
                        &mut *(ptr as *mut $base2)
                    }
                }
            }
        }
    };

    // Three bases
    (
        $(#[$meta:meta])*
        $vis:vis class $name:ident : $base1:ident, $base2:ident, $base3:ident {
            $(
                $(#[$field_meta:meta])*
                $field_vis:vis $field_name:ident : $field_ty:ty
            ),* $(,)?
        }
    ) => {
        $crate::paste! {
            $(#[$meta])*
            #[repr(C)]
            $vis struct $name {
                pub [<vtable_ $base1:snake>]: *const [<$base1 VTable>],
                pub [<vtable_ $base2:snake>]: *const [<$base2 VTable>],
                pub [<vtable_ $base3:snake>]: *const [<$base3 VTable>],
                $(
                    $(#[$field_meta])*
                    $field_vis $field_name: $field_ty,
                )*
            }

            impl $name {
                #[inline]
                pub fn [<as_ $base1:snake>](&self) -> &$base1 {
                    unsafe { &*(self as *const Self as *const $base1) }
                }

                #[inline]
                pub fn [<as_ $base1:snake _mut>](&mut self) -> &mut $base1 {
                    unsafe { &mut *(self as *mut Self as *mut $base1) }
                }

                #[inline]
                pub fn [<as_ $base2:snake>](&self) -> &$base2 {
                    unsafe {
                        let ptr = (self as *const Self as *const u8)
                            .add(::std::mem::offset_of!(Self, [<vtable_ $base2:snake>]));
                        &*(ptr as *const $base2)
                    }
                }

                #[inline]
                pub fn [<as_ $base2:snake _mut>](&mut self) -> &mut $base2 {
                    unsafe {
                        let ptr = (self as *mut Self as *mut u8)
                            .add(::std::mem::offset_of!(Self, [<vtable_ $base2:snake>]));
                        &mut *(ptr as *mut $base2)
                    }
                }

                #[inline]
                pub fn [<as_ $base3:snake>](&self) -> &$base3 {
                    unsafe {
                        let ptr = (self as *const Self as *const u8)
                            .add(::std::mem::offset_of!(Self, [<vtable_ $base3:snake>]));
                        &*(ptr as *const $base3)
                    }
                }

                #[inline]
                pub fn [<as_ $base3:snake _mut>](&mut self) -> &mut $base3 {
                    unsafe {
                        let ptr = (self as *mut Self as *mut u8)
                            .add(::std::mem::offset_of!(Self, [<vtable_ $base3:snake>]));
                        &mut *(ptr as *mut $base3)
                    }
                }
            }
        }
    };
}
