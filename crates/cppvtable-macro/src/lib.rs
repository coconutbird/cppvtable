//! Procedural macros for C++ vtable interop (MSVC ABI)
//!
//! Provides:
//! - `#[cpp_interface]` - Define a C++ interface (generates vtable struct)
//! - `#[implement(Interface)]` - Implement an interface for a struct
//!
//! Automatically selects calling convention based on target:
//! - x86: `thiscall` (this in ECX)
//! - x64: `C` (this as first param)
//!
//! Supports explicit slot indices via `#[slot(N)]` attribute on methods.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, FnArg, Ident, ImplItem, ItemImpl, ItemTrait, Lit, Meta, Pat, TraitItem, Type,
    parse_macro_input,
};

/// Parse #[slot(N)] attribute from a list of attributes
fn parse_slot_attr(attrs: &[Attribute]) -> Option<usize> {
    for attr in attrs {
        if attr.path().is_ident("slot") {
            if let Meta::List(meta_list) = &attr.meta {
                let tokens = meta_list.tokens.clone();
                if let Ok(expr) = syn::parse2::<Expr>(tokens) {
                    if let Expr::Lit(expr_lit) = expr {
                        if let Lit::Int(lit_int) = &expr_lit.lit {
                            return lit_int.base10_parse::<usize>().ok();
                        }
                    }
                }
            }
        }
    }
    None
}

/// Convert interface name to vtable field name (snake_case with vtable_ prefix)
/// IFoo -> vtable_i_foo
/// IAnimal -> vtable_i_animal
/// IGearScore -> vtable_i_gear_score
fn interface_to_field_name(interface: &Ident) -> Ident {
    let name = interface.to_string();
    let chars: Vec<char> = name.chars().collect();
    let mut result = String::from("vtable_");

    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            // Add underscore before uppercase if:
            // - Not at start, AND
            // - Either previous char was lowercase, OR
            //   next char exists and is lowercase (handles "IA" in "IAnimal")
            if i > 0 {
                let prev_lower = chars[i - 1].is_lowercase();
                let next_lower = chars.get(i + 1).map_or(false, |c| c.is_lowercase());
                if prev_lower || next_lower {
                    result.push('_');
                }
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }

    format_ident!("{}", result)
}

/// Define a C++ compatible interface.
///
/// This generates:
/// - A vtable struct `{Name}VTable` with function pointers
/// - A base struct `{Name}` with just the vtable pointer
///
/// Supports `#[slot(N)]` attribute to specify explicit vtable slot indices.
/// Gaps are filled with dummy entries that panic if called.
///
/// # Example
/// ```ignore
/// #[cpp_interface]
/// pub trait IAnimal {
///     fn speak(&self);           // slot 0
///     #[slot(5)]
///     fn jump(&self);            // slot 5 (slots 1-4 filled with dummies)
///     fn legs(&self) -> i32;     // slot 6
/// }
/// ```
#[proc_macro_attribute]
pub fn cpp_interface(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemTrait);
    let trait_name = &input.ident;
    let vtable_name = format_ident!("{}VTable", trait_name);
    let vis = &input.vis;

    // Collect methods with their slot indices
    struct MethodInfo {
        slot: usize,
        name: Ident,
        param_names: Vec<Ident>,
        param_types: Vec<Box<Type>>,
        output: syn::ReturnType,
    }

    let mut methods: Vec<MethodInfo> = Vec::new();
    let mut next_slot = 0usize;

    for item in &input.items {
        if let TraitItem::Fn(method) = item {
            let method_name = method.sig.ident.clone();
            let output = method.sig.output.clone();

            // Check for #[slot(N)] attribute
            let slot = if let Some(explicit_slot) = parse_slot_attr(&method.attrs) {
                if explicit_slot < next_slot {
                    panic!(
                        "slot({}) for method '{}' would overlap with previous slots (next available: {})",
                        explicit_slot, method_name, next_slot
                    );
                }
                explicit_slot
            } else {
                next_slot
            };
            next_slot = slot + 1;

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

            methods.push(MethodInfo {
                slot,
                name: method_name,
                param_names: params.iter().map(|(n, _)| n.clone()).collect(),
                param_types: params.iter().map(|(_, t)| t.clone()).collect(),
                output,
            });
        }
    }

    // Sort by slot index
    methods.sort_by_key(|m| m.slot);

    // Generate vtable fields, filling gaps with dummy entries
    let mut vtable_fields = Vec::new();
    let mut wrapper_methods = Vec::new();
    let mut current_slot = 0usize;

    for method in &methods {
        // Fill gaps with dummy entries
        while current_slot < method.slot {
            let dummy_name = format_ident!("__reserved_slot_{}", current_slot);
            vtable_fields.push(quote! {
                #[cfg(target_arch = "x86")]
                pub #dummy_name: unsafe extern "thiscall" fn(this: *mut std::ffi::c_void),
                #[cfg(not(target_arch = "x86"))]
                pub #dummy_name: unsafe extern "C" fn(this: *mut std::ffi::c_void)
            });
            current_slot += 1;
        }

        let method_name = &method.name;
        let param_names = &method.param_names;
        let param_types = &method.param_types;
        let output = &method.output;

        // Generate vtable field (function pointer)
        // x86: thiscall (this in ECX), x64: C calling convention
        vtable_fields.push(quote! {
            #[cfg(target_arch = "x86")]
            pub #method_name: unsafe extern "thiscall" fn(
                this: *mut std::ffi::c_void
                #(, #param_names: #param_types)*
            ) #output,
            #[cfg(not(target_arch = "x86"))]
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

        current_slot += 1;
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
/// - Wrapper functions that cast `this` and call your methods
/// - A `new()` helper or vtable accessor
///
/// Supports `#[slot(N)]` attribute to specify explicit vtable slot indices.
/// Must match the slot indices used in the corresponding `#[cpp_interface]`.
///
/// # Example
/// ```ignore
/// #[implement(IAnimal)]
/// impl Dog {
///     fn speak(&self) { println!("Woof!"); }  // slot 0
///     #[slot(5)]
///     fn jump(&self) { }                       // slot 5
///     fn legs(&self) -> i32 { 4 }              // slot 6
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

    // Collect methods with their slot indices
    struct ImplMethodInfo {
        slot: usize,
        name: Ident,
        param_names: Vec<Ident>,
        param_types: Vec<Box<Type>>,
        output: syn::ReturnType,
        is_mut: bool,
        original: syn::ImplItemFn,
    }

    let mut methods: Vec<ImplMethodInfo> = Vec::new();
    let mut next_slot = 0usize;

    for item in &input.items {
        if let ImplItem::Fn(method) = item {
            let method_name = method.sig.ident.clone();
            let output = method.sig.output.clone();

            // Check for #[slot(N)] attribute
            let slot = if let Some(explicit_slot) = parse_slot_attr(&method.attrs) {
                if explicit_slot < next_slot {
                    panic!(
                        "slot({}) for method '{}' would overlap with previous slots (next available: {})",
                        explicit_slot, method_name, next_slot
                    );
                }
                explicit_slot
            } else {
                next_slot
            };
            next_slot = slot + 1;

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

            // Check if method takes &self or &mut self
            let is_mut = method.sig.inputs.first().map_or(
                false,
                |arg| matches!(arg, FnArg::Receiver(r) if r.mutability.is_some()),
            );

            methods.push(ImplMethodInfo {
                slot,
                name: method_name,
                param_names: params.iter().map(|(n, _)| n.clone()).collect(),
                param_types: params.iter().map(|(_, t)| t.clone()).collect(),
                output,
                is_mut,
                original: method.clone(),
            });
        }
    }

    // Sort by slot index
    methods.sort_by_key(|m| m.slot);

    // Derive vtable field name from interface name for this-adjustment
    let vtable_field = interface_to_field_name(&interface_name);

    // Generate wrapper functions and vtable entries, filling gaps
    let mut wrapper_fns = Vec::new();
    let mut vtable_entries = Vec::new();
    let mut original_methods = Vec::new();
    let mut current_slot = 0usize;

    for method in &methods {
        // Fill gaps with dummy panic stubs
        while current_slot < method.slot {
            let dummy_name = format_ident!("__reserved_slot_{}", current_slot);
            let dummy_wrapper = format_ident!("__{}__{}__{}", struct_name, interface_name, dummy_name);

            wrapper_fns.push(quote! {
                #[allow(non_snake_case)]
                #[cfg(target_arch = "x86")]
                unsafe extern "thiscall" fn #dummy_wrapper(_this: *mut std::ffi::c_void) {
                    panic!("Called reserved vtable slot {}", #current_slot);
                }

                #[allow(non_snake_case)]
                #[cfg(not(target_arch = "x86"))]
                unsafe extern "C" fn #dummy_wrapper(_this: *mut std::ffi::c_void) {
                    panic!("Called reserved vtable slot {}", #current_slot);
                }
            });

            vtable_entries.push(quote! {
                #dummy_name: #dummy_wrapper
            });

            current_slot += 1;
        }

        let method_name = &method.name;
        // Include interface name in wrapper to avoid conflicts with multiple inheritance
        let wrapper_name = format_ident!("__{}__{}__{}", struct_name, interface_name, method_name);
        let param_names = &method.param_names;
        let param_types = &method.param_types;
        let output = &method.output;

        // This-adjustment: subtract the offset to get from interface pointer to struct start
        // Uses offset_of! to calculate the offset at compile time
        let this_adjust = quote! {
            let offset = ::std::mem::offset_of!(#struct_type, #vtable_field);
            let adjusted = (this as *mut u8).sub(offset) as *mut #struct_type;
        };

        let this_cast = if method.is_mut {
            quote! { &mut *adjusted }
        } else {
            quote! { &*adjusted }
        };

        // Generate wrapper function
        // x86: thiscall (this in ECX), x64: C calling convention
        wrapper_fns.push(quote! {
            #[allow(non_snake_case)]
            #[cfg(target_arch = "x86")]
            unsafe extern "thiscall" fn #wrapper_name(
                this: *mut std::ffi::c_void
                #(, #param_names: #param_types)*
            ) #output {
                unsafe {
                    #this_adjust
                    let obj = #this_cast;
                    obj.#method_name(#(#param_names),*)
                }
            }

            #[allow(non_snake_case)]
            #[cfg(not(target_arch = "x86"))]
            unsafe extern "C" fn #wrapper_name(
                this: *mut std::ffi::c_void
                #(, #param_names: #param_types)*
            ) #output {
                unsafe {
                    #this_adjust
                    let obj = #this_cast;
                    obj.#method_name(#(#param_names),*)
                }
            }
        });

        // Entry in vtable
        vtable_entries.push(quote! {
            #method_name: #wrapper_name
        });

        // Keep original method (strip #[slot] attribute)
        let mut cleaned_method = method.original.clone();
        cleaned_method.attrs.retain(|a| !a.path().is_ident("slot"));
        original_methods.push(cleaned_method);

        current_slot += 1;
    }

    // Include interface name in vtable static name to support multiple interfaces
    let vtable_static_name = format_ident!(
        "__{}_{}_VTABLE",
        struct_name.to_string().to_uppercase(),
        interface_name.to_string().to_uppercase()
    );

    // Generate const name matching field naming convention: vtable_i_foo -> VTABLE_I_FOO
    let vtable_const_name = format_ident!(
        "{}",
        vtable_field.to_string().to_uppercase()
    );

    let expanded = quote! {
        // The wrapper functions (private)
        #(#wrapper_fns)*

        // Static vtable instance
        static #vtable_static_name: #vtable_name = #vtable_name {
            #(#vtable_entries),*
        };

        // Original impl with methods + vtable const accessor
        impl #struct_type {
            /// Pointer to the vtable for this interface implementation.
            /// Use this when constructing the struct.
            pub const #vtable_const_name: *const #vtable_name = &#vtable_static_name;

            #(#original_methods)*
        }
    };

    TokenStream::from(expanded)
}
