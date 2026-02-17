//! Procedural macros for C++ vtable interop (MSVC ABI)
//!
//! Provides:
//! - `#[cppvtable]` - Define a C++ interface (generates vtable struct)
//! - `#[cppvtable_impl(Interface)]` - Implement an interface for a struct
//!
//! Automatically selects calling convention based on target:
//! - x86: `thiscall` (this in ECX)
//! - x64: `C` (this as first param)
//!
//! Supports explicit slot indices via `#[slot(N)]` attribute on methods.
//!
//! ## RTTI Support
//!
//! Both macros generate RTTI (Runtime Type Information) compatible with MSVC/Itanium ABI:
//! - `#[cppvtable]` generates a unique interface ID
//! - `#[cppvtable_impl]` generates TypeInfo with interface offsets for this-adjustment

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, FnArg, Ident, ImplItem, ItemImpl, ItemTrait, Lit, Meta, Pat, TraitItem, Type,
    parse_macro_input, spanned::Spanned,
};

/// Parse #[slot(N)] attribute or #[doc(alias = "__slot:N")] from a list of attributes.
///
/// # Slot Attribute Mechanism
///
/// There are two ways to specify slot indices:
///
/// 1. **Direct proc-macro usage**: `#[slot(N)]`
///    Used when applying `#[cppvtable]` or `#[cppvtable_impl]` directly to code.
///
/// 2. **Via declarative macros**: `#[doc(alias = "__slot:N")]`
///    The `define_interface!` macro converts `[N] fn method()` syntax to this form.
///    We use `doc(alias)` as a carrier because custom attributes like `#[slot]` are
///    stripped by the macro expander before they reach proc-macros. The `doc(alias)`
///    attribute survives this process, allowing us to pass slot information through.
///
/// This dual mechanism allows both the clean proc-macro syntax and the declarative
/// macro syntax to work with the same underlying implementation.
fn parse_slot_attr(attrs: &[Attribute]) -> Option<usize> {
    for attr in attrs {
        // Check for #[slot(N)] - direct proc-macro usage
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
        // Check for #[doc(alias = "__slot:N")] - from declarative macros
        if attr.path().is_ident("doc") {
            if let Meta::List(meta_list) = &attr.meta {
                let tokens_str = meta_list.tokens.to_string();
                // Parse "alias = \"__slot:N\""
                if let Some(alias_val) = tokens_str.strip_prefix("alias = \"") {
                    if let Some(slot_str) = alias_val.strip_prefix("__slot:") {
                        if let Some(num_str) = slot_str.strip_suffix('"') {
                            if let Ok(slot) = num_str.parse::<usize>() {
                                return Some(slot);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Parse slot overrides from attribute: slots(method_name = N, ...)
/// Returns a map of method name -> slot index
fn parse_slot_overrides(attr: TokenStream) -> std::collections::HashMap<String, usize> {
    let mut overrides = std::collections::HashMap::new();
    let attr_str = attr.to_string();

    // Parse "slots(method1 = 3, method2 = 5, ...)"
    if let Some(inner) = attr_str.strip_prefix("slots").map(|s| s.trim()) {
        if let Some(inner) = inner.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            for assignment in inner.split(',') {
                let parts: Vec<&str> = assignment.split('=').collect();
                if parts.len() == 2 {
                    let method = parts[0].trim();
                    if let Ok(slot) = parts[1].trim().parse::<usize>() {
                        overrides.insert(method.to_string(), slot);
                    }
                }
            }
        }
    }

    overrides
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

/// Internal implementation of cppvtable
fn cppvtable_internal(attr: TokenStream, input: ItemTrait) -> Result<TokenStream2, syn::Error> {
    let trait_name = &input.ident;
    let vtable_name = format_ident!("{}VTable", trait_name);
    let vis = &input.vis;

    // Parse slot mappings from attribute: slots(method_name = N, ...)
    let slot_overrides = parse_slot_overrides(attr);

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

            // Check for slot override from attribute, then #[slot(N)] on method
            let slot = if let Some(&explicit_slot) = slot_overrides.get(&method_name.to_string()) {
                if explicit_slot < next_slot {
                    return Err(syn::Error::new(
                        method_name.span(),
                        format!(
                            "slot({}) for method '{}' would overlap with previous slots (next available: {})",
                            explicit_slot, method_name, next_slot
                        ),
                    ));
                }
                explicit_slot
            } else if let Some(explicit_slot) = parse_slot_attr(&method.attrs) {
                if explicit_slot < next_slot {
                    return Err(syn::Error::new(
                        method_name.span(),
                        format!(
                            "slot({}) for method '{}' would overlap with previous slots (next available: {})",
                            explicit_slot, method_name, next_slot
                        ),
                    ));
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
        // Note: The function is already `unsafe fn`, so no inner `unsafe` block needed
        wrapper_methods.push(quote! {
            #[inline]
            pub unsafe fn #method_name(&mut self #(, #param_names: #param_types)*) #output {
                ((*self.vtable).#method_name)(
                    self as *mut Self as *mut std::ffi::c_void
                    #(, #param_names)*
                )
            }
        });

        current_slot += 1;
    }

    // Generate interface ID names
    // IFoo -> IID_IFOO (static) and iid_i_foo() (helper function)
    let iid_static_name = format_ident!("IID_{}", trait_name.to_string().to_uppercase());

    let expanded = quote! {
        /// Unique interface ID for RTTI (address of this static serves as ID)
        #[doc(hidden)]
        #vis static #iid_static_name: u8 = 0;

        /// VTable struct for #trait_name
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
            /// Get the interface ID pointer for this interface type (const-compatible)
            #[inline]
            #[must_use]
            pub const fn interface_id_ptr() -> *const u8 {
                &#iid_static_name as *const u8
            }

            /// Get the interface ID for this interface type as usize
            #[inline]
            #[must_use]
            pub fn interface_id() -> usize {
                Self::interface_id_ptr() as usize
            }

            /// Get the vtable
            #[inline]
            #[must_use]
            pub fn vtable(&self) -> &#vtable_name {
                unsafe { &*self.vtable }
            }

            /// Wrap a raw C++ pointer for calling methods.
            ///
            /// # Safety
            ///
            /// - `ptr` must point to a valid C++ object with a compatible vtable layout
            /// - The returned reference must not outlive the underlying C++ object
            /// - The caller is responsible for ensuring the lifetime `'a` is valid;
            ///   the C++ object must remain alive and unmoved for the duration of `'a`
            /// - No mutable references to the same object may exist concurrently
            ///
            /// # Implementation Notes
            ///
            /// Uses `read_volatile` and `compiler_fence` to prevent the compiler from
            /// optimizing away the pointer indirection, which is necessary when the
            /// pointer comes from C++ code that the Rust compiler cannot reason about.
            #[inline]
            pub unsafe fn from_ptr<'a>(ptr: *mut std::ffi::c_void) -> &'a Self {
                std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
                let ptr = std::ptr::read_volatile(&ptr);
                &*(ptr as *const Self)
            }

            /// Wrap a raw C++ pointer for calling methods (mutable).
            ///
            /// # Safety
            ///
            /// - `ptr` must point to a valid C++ object with a compatible vtable layout
            /// - The returned reference must not outlive the underlying C++ object
            /// - The caller is responsible for ensuring the lifetime `'a` is valid;
            ///   the C++ object must remain alive and unmoved for the duration of `'a`
            /// - No other references (mutable or immutable) to the same object may
            ///   exist concurrently
            ///
            /// # Implementation Notes
            ///
            /// Uses `read_volatile` and `compiler_fence` to prevent the compiler from
            /// optimizing away the pointer indirection, which is necessary when the
            /// pointer comes from C++ code that the Rust compiler cannot reason about.
            #[inline]
            pub unsafe fn from_ptr_mut<'a>(ptr: *mut std::ffi::c_void) -> &'a mut Self {
                std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
                let ptr = std::ptr::read_volatile(&ptr);
                &mut *(ptr as *mut Self)
            }

            #(#wrapper_methods)*
        }
    };

    Ok(expanded)
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
/// #[cppvtable]
/// pub trait IAnimal {
///     fn speak(&self);           // slot 0
///     #[slot(5)]
///     fn jump(&self);            // slot 5 (slots 1-4 filled with dummies)
///     fn legs(&self) -> i32;     // slot 6
/// }
/// ```
#[proc_macro_attribute]
pub fn cppvtable(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemTrait);
    match cppvtable_internal(attr, input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Internal implementation of cppvtable_impl
fn cppvtable_impl_impl(interface_name: Ident, input: ItemImpl) -> Result<TokenStream2, syn::Error> {
    let struct_type = &input.self_ty;
    let vtable_name = format_ident!("{}VTable", interface_name);

    // Extract struct name for generating identifiers
    let struct_name = match struct_type.as_ref() {
        Type::Path(type_path) => type_path.path.segments.last().unwrap().ident.clone(),
        _ => return Err(syn::Error::new(struct_type.span(), "Expected a type path")),
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
                    return Err(syn::Error::new(
                        method_name.span(),
                        format!(
                            "slot({}) for method '{}' would overlap with previous slots (next available: {})",
                            explicit_slot, method_name, next_slot
                        ),
                    ));
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
            let dummy_wrapper =
                format_ident!("__{}__{}__{}", struct_name, interface_name, dummy_name);

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
    let vtable_const_name = format_ident!("{}", vtable_field.to_string().to_uppercase());

    // Generate RTTI InterfaceInfo const name: IFoo -> INTERFACE_INFO_I_FOO
    let interface_info_const_name = format_ident!(
        "INTERFACE_INFO_{}",
        vtable_field
            .to_string()
            .trim_start_matches("vtable_")
            .to_uppercase()
    );

    let expanded = quote! {
        // The wrapper functions (private)
        #(#wrapper_fns)*

        // Static vtable instance
        static #vtable_static_name: #vtable_name = #vtable_name {
            #(#vtable_entries),*
        };

        // Original impl with methods + vtable const accessor + RTTI
        impl #struct_type {
            /// Pointer to the vtable for this interface implementation.
            /// Use this when constructing the struct.
            pub const #vtable_const_name: *const #vtable_name = &#vtable_static_name;

            /// RTTI: Interface info for this interface implementation.
            /// Contains interface ID and byte offset from struct start.
            pub const #interface_info_const_name: cppvtable::InterfaceInfo = cppvtable::InterfaceInfo {
                interface_id: #interface_name::interface_id_ptr(),
                offset: ::std::mem::offset_of!(Self, #vtable_field) as isize,
            };

            #(#original_methods)*
        }
    };

    Ok(expanded)
}

/// Implement a C++ interface for a struct.
///
/// This generates:
/// - Static vtable instance
/// - Wrapper functions that cast `this` and call your methods
/// - A `new()` helper or vtable accessor
///
/// Supports `#[slot(N)]` attribute to specify explicit vtable slot indices.
/// Must match the slot indices used in the corresponding `#[cppvtable]`.
///
/// # Example
/// ```ignore
/// #[cppvtable_impl(IAnimal)]
/// impl Dog {
///     fn speak(&self) { println!("Woof!"); }  // slot 0
///     #[slot(5)]
///     fn jump(&self) { }                       // slot 5
///     fn legs(&self) -> i32 { 4 }              // slot 6
/// }
/// ```
#[proc_macro_attribute]
pub fn cppvtable_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let interface_name = parse_macro_input!(attr as Ident);
    let input = parse_macro_input!(item as ItemImpl);
    match cppvtable_impl_impl(interface_name, input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
