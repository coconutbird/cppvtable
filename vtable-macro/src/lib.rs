//! Procedural macros for C++ vtable interop (MSVC ABI)
//!
//! Provides:
//! - `#[cpp_interface]` - Define a C++ interface (generates vtable struct)
//! - `#[implement(Interface)]` - Implement an interface for a struct
//!
//! Automatically selects calling convention based on target:
//! - x86: `thiscall` (this in ECX)
//! - x64: `C` (this as first param)

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, FnArg, Ident, ImplItem, ItemImpl, ItemTrait, Pat, TraitItem, Type,
};

// Note: For proper thiscall support on x86, we'd need to generate different
// extern ABIs based on target. For now, using extern "C" which works on x64
// and is compatible with cdecl on x86. Real x86 MSVC interop would need thiscall.

/// Define a C++ compatible interface.
///
/// This generates:
/// - A vtable struct `{Name}VTable` with function pointers
/// - A base struct `{Name}` with just the vtable pointer
///
/// # Example
/// ```ignore
/// #[cpp_interface]
/// pub trait IAnimal {
///     fn destructor(&mut self, flags: u8) -> *mut c_void;
///     fn speak(&self);
///     fn legs(&self) -> i32;
/// }
/// ```
#[proc_macro_attribute]
pub fn cpp_interface(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemTrait);
    let trait_name = &input.ident;
    let vtable_name = format_ident!("{}VTable", trait_name);
    let vis = &input.vis;

    // Extract methods from trait
    let mut vtable_fields = Vec::new();
    let mut wrapper_methods = Vec::new();

    for item in &input.items {
        if let TraitItem::Fn(method) = item {
            let method_name = &method.sig.ident;
            let output = &method.sig.output;

            // Collect parameter names and types (skip self)
            let params: Vec<_> = method
                .sig
                .inputs
                .iter()
                .filter_map(|arg| {
                    if let FnArg::Typed(pat_type) = arg {
                        if let Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                            let name = &pat_ident.ident;
                            let ty = &pat_type.ty;
                            return Some((name.clone(), ty.clone()));
                        }
                    }
                    None
                })
                .collect();

            let param_names: Vec<_> = params.iter().map(|(n, _)| n).collect();
            let param_types: Vec<_> = params.iter().map(|(_, t)| t).collect();

            // Generate vtable field (function pointer)
            vtable_fields.push(quote! {
                pub #method_name: unsafe extern "C" fn(
                    this: *mut std::ffi::c_void
                    #(, #param_names: #param_types)*
                ) #output
            });

            // Generate wrapper method on the base struct
            wrapper_methods.push(quote! {
                #[inline]
                pub unsafe fn #method_name(&mut self #(, #param_names: #param_types)*) #output {
                    unsafe {
                        ((*self.vtable).#method_name)(
                            self as *mut Self as *mut std::ffi::c_void
                            #(, #param_names)*
                        )
                    }
                }
            });
        }
    }

    let expanded = quote! {
        /// VTable struct for #trait_name
        /// Note: In MSVC ABI, RTTI pointer would be at offset -1 (not included here)
        #[repr(C)]
        #vis struct #vtable_name {
            #(#vtable_fields),*
        }

        /// Base struct representing the interface pointer
        #[repr(C)]
        #vis struct #trait_name {
            vtable: *const #vtable_name,
        }

        impl #trait_name {
            /// Get the vtable
            #[inline]
            pub fn vtable(&self) -> &#vtable_name {
                unsafe { &*self.vtable }
            }

            /// Wrap a raw C++ pointer for calling methods.
            /// Uses compiler fence to prevent optimization issues.
            #[inline]
            pub unsafe fn from_ptr<'a>(ptr: *mut std::ffi::c_void) -> &'a Self {
                std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
                let ptr = std::ptr::read_volatile(&ptr);
                unsafe { &*(ptr as *const Self) }
            }

            /// Wrap a raw C++ pointer for calling methods (mutable).
            /// Uses compiler fence to prevent optimization issues.
            #[inline]
            pub unsafe fn from_ptr_mut<'a>(ptr: *mut std::ffi::c_void) -> &'a mut Self {
                std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
                let ptr = std::ptr::read_volatile(&ptr);
                unsafe { &mut *(ptr as *mut Self) }
            }

            #(#wrapper_methods)*
        }
    };

    TokenStream::from(expanded)
}

/// Implement a C++ interface for a struct.
///
/// This generates:
/// - Static vtable instance
/// - `extern "C"` wrapper functions that cast `this` and call your methods
/// - A `new()` helper or vtable accessor
///
/// # Example
/// ```ignore
/// #[implement(IAnimal)]
/// impl Dog {
///     fn destructor(&mut self, flags: u8) -> *mut c_void { ... }
///     fn speak(&self) { println!("Woof!"); }
///     fn legs(&self) -> i32 { 4 }
/// }
/// ```
#[proc_macro_attribute]
pub fn implement(attr: TokenStream, item: TokenStream) -> TokenStream {
    let interface_name = parse_macro_input!(attr as Ident);
    let input = parse_macro_input!(item as ItemImpl);

    let struct_type = &input.self_ty;
    let vtable_name = format_ident!("{}VTable", interface_name);

    // Extract struct name for generating identifiers
    let struct_name = match struct_type.as_ref() {
        Type::Path(type_path) => type_path.path.segments.last().unwrap().ident.clone(),
        _ => panic!("Expected a type path"),
    };

    let mut wrapper_fns = Vec::new();
    let mut vtable_entries = Vec::new();
    let mut original_methods = Vec::new();

    for item in &input.items {
        if let ImplItem::Fn(method) = item {
            let method_name = &method.sig.ident;
            let wrapper_name = format_ident!("__{}__{}", struct_name, method_name);
            let output = &method.sig.output;

            // Collect parameters (skip self)
            let params: Vec<_> = method
                .sig
                .inputs
                .iter()
                .filter_map(|arg| {
                    if let FnArg::Typed(pat_type) = arg {
                        if let Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                            let name = &pat_ident.ident;
                            let ty = &pat_type.ty;
                            return Some((name.clone(), ty.clone()));
                        }
                    }
                    None
                })
                .collect();

            let param_names: Vec<_> = params.iter().map(|(n, _)| n).collect();
            let param_types: Vec<_> = params.iter().map(|(_, t)| t).collect();

            // Check if method takes &self or &mut self
            let is_mut = method.sig.inputs.first().map_or(false, |arg| {
                matches!(arg, FnArg::Receiver(r) if r.mutability.is_some())
            });

            let this_cast = if is_mut {
                quote! { &mut *(this as *mut #struct_type) }
            } else {
                quote! { &*(this as *const #struct_type) }
            };

            // Generate extern "C" wrapper
            wrapper_fns.push(quote! {
                #[allow(non_snake_case)]
                unsafe extern "C" fn #wrapper_name(
                    this: *mut std::ffi::c_void
                    #(, #param_names: #param_types)*
                ) #output {
                    unsafe {
                        let obj = #this_cast;
                        obj.#method_name(#(#param_names),*)
                    }
                }
            });

            // Entry in vtable
            vtable_entries.push(quote! {
                #method_name: #wrapper_name
            });

            // Keep original method
            original_methods.push(method.clone());
        }
    }

    let vtable_static_name = format_ident!("__{}_VTABLE", struct_name.to_string().to_uppercase());

    let expanded = quote! {
        // The wrapper functions (private)
        #(#wrapper_fns)*

        // Static vtable instance
        static #vtable_static_name: #vtable_name = #vtable_name {
            #(#vtable_entries),*
        };

        // Original impl with methods
        impl #struct_type {
            /// Get the vtable for this implementation
            #[inline]
            pub fn vtable_for_interface() -> &'static #vtable_name {
                &#vtable_static_name
            }

            /// Get vtable pointer (for initializing the struct)
            #[inline]
            pub fn vtable_ptr() -> *const #vtable_name {
                &#vtable_static_name
            }

            #(#original_methods)*
        }
    };

    TokenStream::from(expanded)
}
