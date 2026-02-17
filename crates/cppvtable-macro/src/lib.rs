//! Procedural macros for C++ vtable interop (MSVC ABI)
//!
//! Provides:
//! - `#[cppvtable]` - Define a C++ interface (generates vtable struct)
//! - `#[cppvtable_impl(Interface)]` - Implement an interface for a struct
//! - `#[com_interface("guid")]` - Define a COM interface with IUnknown base
//! - `#[com_implement(Interface)]` - Implement a COM interface for a struct
//!
//! ## Calling Conventions
//!
//! **C++ vtables (`cppvtable`):**
//! - x86: `thiscall` (this in ECX)
//! - x64: `C` (this as first param)
//!
//! **COM interfaces (`com_interface`):**
//! - x86: `stdcall` (this on stack)
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

/// Returns the path to the cppvtable crate.
///
/// Returns the path to the cppvtable crate based on the `internal` flag.
///
/// When `internal` is true (used inside the cppvtable crate itself), this returns `crate::`.
/// When `internal` is false (external crates), this returns `cppvtable::`.
fn crate_path(internal: bool) -> TokenStream2 {
    if internal {
        quote! { crate }
    } else {
        quote! { cppvtable }
    }
}

/// Transforms a type to use `$crate::` prefix for cppvtable types.
///
/// This is used when generating declarative macros that will be invoked from user code.
/// In that context, `$crate` resolves to the crate where the macro is defined (cppvtable).
///
/// Types transformed:
/// - `GUID` -> `$crate::GUID`
/// - `HRESULT` -> `$crate::HRESULT`
/// - `c_void` -> `::std::ffi::c_void`
fn qualify_type_for_macro(ty: &Type) -> TokenStream2 {
    match ty {
        Type::Path(type_path) => {
            // Check if it's a simple identifier we need to qualify
            if let Some(ident) = type_path.path.get_ident() {
                let name = ident.to_string();
                match name.as_str() {
                    "GUID" | "HRESULT" => {
                        // These are cppvtable types, need $crate:: prefix
                        return quote! { $crate::#ident };
                    }
                    "c_void" => {
                        // Use fully qualified std path
                        return quote! { ::std::ffi::c_void };
                    }
                    _ => {}
                }
            }
            // Keep other paths as-is
            quote! { #ty }
        }
        Type::Ptr(type_ptr) => {
            let inner = qualify_type_for_macro(&type_ptr.elem);
            if type_ptr.const_token.is_some() {
                if type_ptr.mutability.is_some() {
                    quote! { *const mut #inner }
                } else {
                    quote! { *const #inner }
                }
            } else if type_ptr.mutability.is_some() {
                quote! { *mut #inner }
            } else {
                quote! { *#inner }
            }
        }
        Type::Reference(type_ref) => {
            let inner = qualify_type_for_macro(&type_ref.elem);
            let lifetime = &type_ref.lifetime;
            if type_ref.mutability.is_some() {
                quote! { &#lifetime mut #inner }
            } else {
                quote! { &#lifetime #inner }
            }
        }
        // For other types, keep as-is
        _ => quote! { #ty },
    }
}

// =============================================================================
// Configuration types for vtable generation
// =============================================================================

/// Calling convention for vtable methods
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum CallingConvention {
    /// C++ thiscall: this in ECX on x86, first param on x64
    #[default]
    Thiscall,
    /// COM stdcall: this on stack on x86, first param on x64
    Stdcall,
}

/// Interface ID type
#[derive(Clone, Default)]
enum InterfaceId {
    /// Pointer-based ID (address of a static)
    #[default]
    Pointer,
    /// GUID-based ID (COM style)
    Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    },
    /// No IID generation (for interfaces that define their own IID)
    None,
}

/// Configuration for vtable generation
#[derive(Clone, Default)]
struct VTableConfig {
    /// Calling convention (thiscall vs stdcall)
    calling_convention: CallingConvention,
    /// Base interface to inherit from (e.g., IUnknown)
    /// When set, the generated vtable embeds the base vtable as the first field
    base_interface: Option<syn::Ident>,
    /// Interface ID type
    iid: InterfaceId,
    /// Slot overrides from attribute
    slot_overrides: std::collections::HashMap<String, usize>,
    /// Internal mode: use `crate::` instead of `cppvtable::` for paths
    /// This is used when defining interfaces inside the cppvtable crate itself
    internal: bool,
    /// Skip generating forwarder macros ({interface}_forwarders! and {interface}_base_vtable!)
    /// Use this when the forwarders need to be manually defined (e.g., IUnknown with COM types)
    no_forwarders: bool,
}

impl VTableConfig {
    /// Generate the x86 calling convention token
    fn x86_calling_conv(&self) -> TokenStream2 {
        match self.calling_convention {
            CallingConvention::Thiscall => quote! { "thiscall" },
            CallingConvention::Stdcall => quote! { "stdcall" },
        }
    }
}

/// Configuration for vtable implementation generation
#[derive(Clone, Default)]
struct ImplConfig {
    /// Calling convention (thiscall vs stdcall)
    calling_convention: CallingConvention,
    /// Base interface to inherit from (e.g., IUnknown)
    /// When set, generates forwarders and base vtable entry via the base's macros:
    /// - `{base}_forwarders!` - generates wrapper functions
    /// - `{base}_base_vtable!` - generates base vtable initializer
    /// - `{base}_methods!` - generates method implementations on the struct
    base_interface: Option<syn::Ident>,
    /// First slot index for user methods (e.g., 3 for IUnknown base)
    /// Caller must set this based on the base interface's slot count
    first_slot: usize,
    /// Whether to generate RTTI info
    generate_rtti: bool,
    /// IID constant name for COM (e.g., IID_ICALCULATOR)
    iid_const: Option<syn::Ident>,
    /// Internal mode: use `crate::` instead of `cppvtable::` for paths
    internal: bool,
}

impl ImplConfig {
    /// Generate the x86 calling convention token
    fn x86_calling_conv(&self) -> TokenStream2 {
        match self.calling_convention {
            CallingConvention::Thiscall => quote! { "thiscall" },
            CallingConvention::Stdcall => quote! { "stdcall" },
        }
    }
}

// =============================================================================
// Validation helpers for FFI-safety and C++ compatibility
// =============================================================================

/// Check if a type is known to be non-FFI-safe
fn check_ffi_safe_type(ty: &Type) -> Result<(), String> {
    match ty {
        Type::Path(type_path) => {
            if let Some(segment) = type_path.path.segments.last() {
                let name = segment.ident.to_string();
                match name.as_str() {
                    // Rust-specific types that are not FFI-safe
                    "String" => {
                        return Err(
                            "String is not FFI-safe. Use *const c_char or *const u8 instead".into(),
                        );
                    }
                    "Vec" => {
                        return Err(
                            "Vec<T> is not FFI-safe. Use *const T and a length parameter instead"
                                .into(),
                        );
                    }
                    "Box" => return Err("Box<T> is not FFI-safe. Use *mut T instead".into()),
                    "Rc" | "Arc" => {
                        return Err(format!(
                            "{} is not FFI-safe. Use raw pointers instead",
                            name
                        ));
                    }
                    "Option" => {
                        // Option<NonNull<T>> and Option<fn> are FFI-safe, but Option<T> generally isn't
                        // We'll allow it with a note that the user should be careful
                    }
                    "Result" => return Err(
                        "Result<T, E> is not FFI-safe. Use error codes or out-parameters instead"
                            .into(),
                    ),
                    "str" => {
                        return Err(
                            "str is not FFI-safe. Use *const c_char or *const u8 instead".into(),
                        );
                    }
                    _ => {}
                }
            }
        }
        Type::Reference(type_ref) => {
            // References other than &self/&mut self should be warned about
            // But we can't easily distinguish &self here, so we'll check in the method validation
            let mutability = if type_ref.mutability.is_some() {
                "&mut "
            } else {
                "&"
            };
            return Err(format!(
                "{}T references are not recommended for FFI. Use *const T or *mut T instead. \
                 References have Rust-specific guarantees that C++ won't uphold",
                mutability
            ));
        }
        Type::Slice(_) => {
            return Err(
                "Slices [T] are not FFI-safe. Use *const T and a length parameter instead".into(),
            );
        }
        Type::TraitObject(_) => {
            return Err("Trait objects (dyn Trait) are not FFI-safe".into());
        }
        Type::ImplTrait(_) => {
            return Err("impl Trait is not FFI-safe".into());
        }
        Type::Tuple(tuple) if !tuple.elems.is_empty() => {
            return Err(
                "Non-empty tuples are not FFI-safe. Use a #[repr(C)] struct instead".into(),
            );
        }
        _ => {}
    }
    Ok(())
}

/// Validate a trait method signature for C++ vtable compatibility
fn validate_trait_method(method: &syn::TraitItemFn) -> Result<(), syn::Error> {
    let method_name = &method.sig.ident;
    let span = method_name.span();

    // Check for async
    if method.sig.asyncness.is_some() {
        return Err(syn::Error::new(
            span,
            format!(
                "method '{}': async functions are not supported in C++ vtables",
                method_name
            ),
        ));
    }

    // Check for generics
    if !method.sig.generics.params.is_empty() {
        return Err(syn::Error::new(
            span,
            format!(
                "method '{}': generic methods are not supported in C++ vtables",
                method_name
            ),
        ));
    }

    // Check for self parameter
    let has_self = method
        .sig
        .inputs
        .iter()
        .any(|arg| matches!(arg, FnArg::Receiver(_)));
    if !has_self {
        return Err(syn::Error::new(
            span,
            format!(
                "method '{}': must have &self or &mut self parameter (C++ vtable methods require a this pointer)",
                method_name
            ),
        ));
    }

    // Check self is by reference, not by value
    for arg in &method.sig.inputs {
        if let FnArg::Receiver(receiver) = arg
            && receiver.reference.is_none()
        {
            return Err(syn::Error::new(
                receiver.self_token.span(),
                format!(
                    "method '{}': self by value is not supported. Use &self or &mut self instead",
                    method_name
                ),
            ));
        }
    }

    // Check parameter types for FFI safety
    for arg in &method.sig.inputs {
        if let FnArg::Typed(pat_type) = arg
            && let Err(msg) = check_ffi_safe_type(&pat_type.ty)
        {
            return Err(syn::Error::new(
                pat_type.ty.span(),
                format!("method '{}': {}", method_name, msg),
            ));
        }
    }

    // Check return type for FFI safety
    if let syn::ReturnType::Type(_, ty) = &method.sig.output
        && let Err(msg) = check_ffi_safe_type(ty)
    {
        return Err(syn::Error::new(
            ty.span(),
            format!("method '{}': return type - {}", method_name, msg),
        ));
    }

    Ok(())
}

/// Validate an impl method signature for C++ vtable compatibility
fn validate_impl_method(method: &syn::ImplItemFn) -> Result<(), syn::Error> {
    let method_name = &method.sig.ident;
    let span = method_name.span();

    // Check for async
    if method.sig.asyncness.is_some() {
        return Err(syn::Error::new(
            span,
            format!(
                "method '{}': async functions are not supported in C++ vtables",
                method_name
            ),
        ));
    }

    // Check for generics
    if !method.sig.generics.params.is_empty() {
        return Err(syn::Error::new(
            span,
            format!(
                "method '{}': generic methods are not supported in C++ vtables",
                method_name
            ),
        ));
    }

    // Check for self parameter
    let has_self = method
        .sig
        .inputs
        .iter()
        .any(|arg| matches!(arg, FnArg::Receiver(_)));
    if !has_self {
        return Err(syn::Error::new(
            span,
            format!(
                "method '{}': must have &self or &mut self parameter (C++ vtable methods require a this pointer)",
                method_name
            ),
        ));
    }

    // Check self is by reference, not by value
    for arg in &method.sig.inputs {
        if let FnArg::Receiver(receiver) = arg
            && receiver.reference.is_none()
        {
            return Err(syn::Error::new(
                receiver.self_token.span(),
                format!(
                    "method '{}': self by value is not supported. Use &self or &mut self instead",
                    method_name
                ),
            ));
        }
    }

    // Check parameter types for FFI safety
    for arg in &method.sig.inputs {
        if let FnArg::Typed(pat_type) = arg
            && let Err(msg) = check_ffi_safe_type(&pat_type.ty)
        {
            return Err(syn::Error::new(
                pat_type.ty.span(),
                format!("method '{}': {}", method_name, msg),
            ));
        }
    }

    // Check return type for FFI safety
    if let syn::ReturnType::Type(_, ty) = &method.sig.output
        && let Err(msg) = check_ffi_safe_type(ty)
    {
        return Err(syn::Error::new(
            ty.span(),
            format!("method '{}': return type - {}", method_name, msg),
        ));
    }

    Ok(())
}

/// Validate a trait definition for C++ vtable compatibility
fn validate_trait(input: &ItemTrait) -> Result<(), syn::Error> {
    // Note: Generic traits are supported. When a trait has generic type parameters
    // (e.g., `trait IInArchive<T>`), the generated vtable function pointers will use
    // `*mut T` instead of `*mut c_void` for type-safe function pointers.
    // This is useful for COM interfaces where T represents the implementing struct type.

    // Validate each method
    for item in &input.items {
        if let TraitItem::Fn(method) = item {
            validate_trait_method(method)?;
        }
    }

    Ok(())
}

/// Validate an impl block for C++ vtable compatibility
fn validate_impl(input: &ItemImpl) -> Result<(), syn::Error> {
    // Check for generics on the impl
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "generic impl blocks are not supported in C++ vtables",
        ));
    }

    // Validate each method
    for item in &input.items {
        if let ImplItem::Fn(method) = item {
            validate_impl_method(method)?;
        }
    }

    Ok(())
}

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
        if attr.path().is_ident("slot")
            && let Meta::List(meta_list) = &attr.meta
            && let Ok(Expr::Lit(expr_lit)) = syn::parse2::<Expr>(meta_list.tokens.clone())
            && let Lit::Int(lit_int) = &expr_lit.lit
        {
            return lit_int.base10_parse::<usize>().ok();
        }
        // Check for #[doc(alias = "__slot:N")] - from declarative macros
        if attr.path().is_ident("doc")
            && let Meta::List(meta_list) = &attr.meta
        {
            let tokens_str = meta_list.tokens.to_string();
            // Parse "alias = \"__slot:N\""
            if let Some(alias_val) = tokens_str.strip_prefix("alias = \"")
                && let Some(slot_str) = alias_val.strip_prefix("__slot:")
                && let Some(num_str) = slot_str.strip_suffix('"')
                && let Ok(slot) = num_str.parse::<usize>()
            {
                return Some(slot);
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
                let next_lower = chars.get(i + 1).is_some_and(|c| c.is_lowercase());
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

/// Internal implementation of cppvtable - unified for both C++ and COM interfaces
fn cppvtable_internal(config: VTableConfig, input: ItemTrait) -> Result<TokenStream2, syn::Error> {
    // Validate trait for C++ vtable compatibility
    validate_trait(&input)?;

    let trait_name = &input.ident;
    let vtable_name = format_ident!("{}VTable", trait_name);
    let vis = &input.vis;
    let x86_cc = config.x86_calling_conv();

    // Extract generics from the trait for generic interface support
    // When a trait has generic type parameters (e.g., `trait IInArchive<T>`),
    // the vtable function pointers will use `*mut T` instead of `*mut c_void`
    let generics = &input.generics;
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let has_type_params = generics.type_params().next().is_some();

    // Determine the self pointer type for vtable function pointers
    // When generics are present, use *mut T (first type param) for type-safe function pointers
    // Otherwise, use *mut std::ffi::c_void for compatibility
    let self_ptr_type = if has_type_params {
        let first_type_param = generics.type_params().next().unwrap();
        let t_ident = &first_type_param.ident;
        quote! { *mut #t_ident }
    } else {
        quote! { *mut std::ffi::c_void }
    };

    // Handle base interface inheritance
    // When a base is specified:
    // - Embed the base vtable struct as the first field
    // - Own method slots start at 0 (relative to derived portion)
    // - Total slot count = base slot count + own method count
    let krate = crate_path(config.internal);
    let base_vtable_field = config.base_interface.as_ref().map(|base_ident| {
        quote! {
            /// Inherited base interface vtable
            pub base: <#base_ident as #krate::VTableLayout>::VTable
        }
    });

    // When extending, slot indices are relative to the derived interface
    // (slot 0 = first method after base)
    let first_slot = 0usize;

    // Collect methods with their slot indices
    struct MethodInfo {
        slot: usize,
        name: Ident,
        param_names: Vec<Ident>,
        param_types: Vec<Type>,
        output: syn::ReturnType,
    }

    let mut methods: Vec<MethodInfo> = Vec::new();
    let mut next_slot = first_slot;

    for item in &input.items {
        if let TraitItem::Fn(method) = item {
            let method_name = method.sig.ident.clone();
            let output = method.sig.output.clone();

            // Check for slot override from attribute, then #[slot(N)] on method
            let slot = if let Some(&explicit_slot) =
                config.slot_overrides.get(&method_name.to_string())
            {
                if explicit_slot < first_slot {
                    return Err(syn::Error::new(
                        method_name.span(),
                        format!(
                            "slot({}) for method '{}' conflicts with base interface methods (first available: {})",
                            explicit_slot, method_name, first_slot
                        ),
                    ));
                }
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
                if explicit_slot < first_slot {
                    return Err(syn::Error::new(
                        method_name.span(),
                        format!(
                            "slot({}) for method '{}' conflicts with base interface methods (first available: {})",
                            explicit_slot, method_name, first_slot
                        ),
                    ));
                }
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
                    if let FnArg::Typed(pat_type) = arg
                        && let Pat::Ident(pat_ident) = pat_type.pat.as_ref()
                    {
                        let name = &pat_ident.ident;
                        let ty = pat_type.ty.as_ref();
                        return Some((name.clone(), ty.clone()));
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
                pub #dummy_name: unsafe extern #x86_cc fn(this: #self_ptr_type),
                #[cfg(not(target_arch = "x86"))]
                pub #dummy_name: unsafe extern "C" fn(this: #self_ptr_type)
            });
            current_slot += 1;
        }

        let method_name = &method.name;
        let param_names = &method.param_names;
        let param_types = &method.param_types;
        let output = &method.output;

        // Generate vtable field (function pointer) using configured calling convention
        // Uses self_ptr_type: *mut T for generic interfaces, *mut c_void for non-generic
        vtable_fields.push(quote! {
            #[cfg(target_arch = "x86")]
            pub #method_name: unsafe extern #x86_cc fn(
                this: #self_ptr_type
                #(, #param_names: #param_types)*
            ) #output,
            #[cfg(not(target_arch = "x86"))]
            pub #method_name: unsafe extern "C" fn(
                this: #self_ptr_type
                #(, #param_names: #param_types)*
            ) #output
        });

        // Generate wrapper method on the base struct
        // Cast self to the appropriate pointer type (c_void or T)
        wrapper_methods.push(quote! {
            #[inline]
            pub unsafe fn #method_name(&mut self #(, #param_names: #param_types)*) #output {
                ((*self.vtable).#method_name)(
                    self as *mut Self as #self_ptr_type
                    #(, #param_names)*
                )
            }
        });

        current_slot += 1;
    }

    // Total slot count for VTableLayout
    let total_slot_count = current_slot;

    // Generate interface ID based on config
    let iid_static_name = format_ident!("IID_{}", trait_name.to_string().to_uppercase());

    // Generate IID definition and methods based on config
    let (iid_definition, iid_methods) = match &config.iid {
        InterfaceId::Pointer => {
            let def = quote! {
                /// Unique interface ID for RTTI (address of this static serves as ID)
                #[doc(hidden)]
                #vis static #iid_static_name: u8 = 0;
            };
            let methods = quote! {
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
            };
            (def, methods)
        }
        InterfaceId::Guid {
            data1,
            data2,
            data3,
            data4,
        } => {
            let d4_0 = data4[0];
            let d4_1 = data4[1];
            let d4_2 = data4[2];
            let d4_3 = data4[3];
            let d4_4 = data4[4];
            let d4_5 = data4[5];
            let d4_6 = data4[6];
            let d4_7 = data4[7];
            // Use make_guid helper function for windows-compat compatibility
            let def = quote! {
                /// COM Interface ID (GUID) for this interface
                #vis const #iid_static_name: #krate::GUID = #krate::make_guid(
                    #data1,
                    #data2,
                    #data3,
                    [#d4_0, #d4_1, #d4_2, #d4_3, #d4_4, #d4_5, #d4_6, #d4_7],
                );
            };
            let methods = quote! {
                /// Get the interface ID (GUID) for this COM interface
                #[inline]
                #[must_use]
                pub const fn iid() -> &'static #krate::GUID {
                    &#iid_static_name
                }
            };
            (def, methods)
        }
        InterfaceId::None => {
            // No IID generation - user defines their own IID externally
            (quote! {}, quote! {})
        }
    };

    // Generate the slot count expression
    // If we have a base, total = base slot count + own slot count
    let own_slot_count = total_slot_count;
    let slot_count_expr = if let Some(ref base_ident) = config.base_interface {
        quote! { <#base_ident as #krate::VTableLayout>::SLOT_COUNT + #own_slot_count }
    } else {
        quote! { #own_slot_count }
    };

    // Generate vtable struct with optional base field and generic parameters
    let vtable_struct = if let Some(ref base_field) = base_vtable_field {
        quote! {
            /// VTable struct for #trait_name
            #[repr(C)]
            #vis struct #vtable_name #impl_generics #where_clause {
                #base_field,
                #(#vtable_fields),*
            }
        }
    } else {
        quote! {
            /// VTable struct for #trait_name
            #[repr(C)]
            #vis struct #vtable_name #impl_generics #where_clause {
                #(#vtable_fields),*
            }
        }
    };

    // Generate IUnknown forwarding methods if extending IUnknown
    let iunknown_wrappers = if config
        .base_interface
        .as_ref()
        .is_some_and(|name| name == "IUnknown")
    {
        quote! {
            /// Query for another interface by GUID (forwarded to base IUnknown)
            ///
            /// # Safety
            /// - `riid` must point to a valid GUID
            /// - `ppv` must point to a valid pointer location
            #[inline]
            pub unsafe fn query_interface(
                &self,
                riid: *const #krate::GUID,
                ppv: *mut *mut std::ffi::c_void,
            ) -> #krate::HRESULT {
                unsafe {
                    ((*self.vtable).base.query_interface)(
                        self as *const Self as *mut std::ffi::c_void,
                        riid,
                        ppv,
                    )
                }
            }

            /// Increment reference count (forwarded to base IUnknown)
            #[inline]
            pub unsafe fn add_ref(&self) -> u32 {
                unsafe {
                    ((*self.vtable).base.add_ref)(self as *const Self as *mut std::ffi::c_void)
                }
            }

            /// Decrement reference count (forwarded to base IUnknown)
            #[inline]
            pub unsafe fn release(&self) -> u32 {
                unsafe {
                    ((*self.vtable).base.release)(self as *const Self as *mut std::ffi::c_void)
                }
            }
        }
    } else {
        quote! {}
    };

    // Generate {interface}_forwarders! and {interface}_base_vtable! macros
    // These allow this interface to be used as a base for other interfaces
    let interface_lower = trait_name.to_string().to_lowercase();
    let forwarders_macro_name = format_ident!("{}_forwarders", interface_lower);
    let base_vtable_macro_name = format_ident!("{}_base_vtable", interface_lower);

    // Generate wrapper function code for each method in the forwarders macro
    let mut forwarder_wrappers = Vec::new();
    let mut vtable_entries = Vec::new();

    for method in &methods {
        let method_name = &method.name;
        let method_name_str = method_name.to_string();
        let param_names = &method.param_names;
        let param_types = &method.param_types;
        let output = &method.output;

        // Qualify types for use in declarative macro context
        // This transforms GUID -> $crate::GUID, HRESULT -> $crate::HRESULT, etc.
        let qualified_param_types: Vec<_> =
            param_types.iter().map(qualify_type_for_macro).collect();

        // Build parameter list for function signature with qualified types
        let params_with_types: Vec<_> = param_names
            .iter()
            .zip(qualified_param_types.iter())
            .map(|(name, ty)| quote! { #name: #ty })
            .collect();

        // Qualify return type too
        let qualified_output = match output {
            syn::ReturnType::Default => quote! {},
            syn::ReturnType::Type(arrow, ty) => {
                let qualified_ty = qualify_type_for_macro(ty);
                quote! { #arrow #qualified_ty }
            }
        };

        // Build the method call arguments (just parameter names)
        let call_args: Vec<_> = param_names.iter().map(|name| quote! { #name }).collect();

        // Generate wrapper functions for x86 and x64
        // Uses paste! for identifier concatenation with $struct_name and $interface_name
        forwarder_wrappers.push(quote! {
            #[allow(non_snake_case)]
            #[cfg(target_arch = "x86")]
            unsafe extern #x86_cc fn [<__ $struct_name __ $interface_name __ #method_name_str>](
                this: *mut ::std::ffi::c_void
                #(, #params_with_types)*
            ) #qualified_output {
                unsafe {
                    let offset = ::std::mem::offset_of!($struct_type, $vtable_field);
                    let adjusted = (this as *mut u8).sub(offset) as *mut $struct_type;
                    (*adjusted).#method_name(#(#call_args),*)
                }
            }

            #[allow(non_snake_case)]
            #[cfg(not(target_arch = "x86"))]
            unsafe extern "C" fn [<__ $struct_name __ $interface_name __ #method_name_str>](
                this: *mut ::std::ffi::c_void
                #(, #params_with_types)*
            ) #qualified_output {
                unsafe {
                    let offset = ::std::mem::offset_of!($struct_type, $vtable_field);
                    let adjusted = (this as *mut u8).sub(offset) as *mut $struct_type;
                    (*adjusted).#method_name(#(#call_args),*)
                }
            }
        });

        // Generate vtable entry for base_vtable macro
        vtable_entries.push(quote! {
            #method_name: [<__ $struct_name __ $interface_name __ #method_name_str>]
        });
    }

    // Generate the forwarders macro and base_vtable macro
    // Skip if no_forwarders is set (e.g., for IUnknown where manual forwarders are needed)
    let (forwarders_macro, base_vtable_macro) = if config.no_forwarders {
        (quote! {}, quote! {})
    } else if let Some(ref base_ident) = config.base_interface {
        let base_lower = base_ident.to_string().to_lowercase();
        let base_forwarders_macro = format_ident!("{}_forwarders", base_lower);
        let parent_base_vtable_macro = format_ident!("{}_base_vtable", base_lower);
        (
            quote! {
                /// Auto-generated forwarders macro for #trait_name.
                ///
                /// This macro generates wrapper functions that adjust the `this` pointer
                /// and forward calls to the implementing struct's methods.
                /// Also invokes the base interface's forwarders macro.
                ///
                /// # Parameters
                /// - `$struct_name`: The implementing struct name (e.g., `Calculator`)
                /// - `$struct_type`: The implementing struct type (e.g., `Calculator` or `Calculator<T>`)
                /// - `$interface_name`: The interface being implemented (e.g., `ICalculator`)
                /// - `$vtable_field`: The vtable pointer field name (e.g., `vtable_i_calculator`)
                /// - `$iid_const`: The IID constant for the interface (unused but kept for consistency)
                #[macro_export]
                macro_rules! #forwarders_macro_name {
                    ($struct_name:ident, $struct_type:ty, $interface_name:ident, $vtable_field:ident, $iid_const:ident) => {
                        // First invoke base interface's forwarders
                        $crate::#base_forwarders_macro!($struct_name, $struct_type, $interface_name, $vtable_field, $iid_const);

                        // Then generate our own forwarders
                        $crate::paste! {
                            #(#forwarder_wrappers)*
                        }
                    };
                }
            },
            quote! {
                /// Auto-generated base vtable initializer macro for #trait_name.
                ///
                /// Returns an expression that creates `#vtable_name { base: ..., ... }` with the wrapper function pointers.
                /// Recursively invokes the parent interface's base_vtable macro for the base field.
                #[macro_export]
                macro_rules! #base_vtable_macro_name {
                    ($struct_name:ident, $interface_name:ident) => {
                        $crate::paste! {
                            #vtable_name {
                                base: $crate::#parent_base_vtable_macro!($struct_name, $interface_name),
                                #(#vtable_entries),*
                            }
                        }
                    };
                }
            },
        )
    } else {
        (
            quote! {
                /// Auto-generated forwarders macro for #trait_name.
                ///
                /// This macro generates wrapper functions that adjust the `this` pointer
                /// and forward calls to the implementing struct's methods.
                ///
                /// # Parameters
                /// - `$struct_name`: The implementing struct name (e.g., `Calculator`)
                /// - `$struct_type`: The implementing struct type (e.g., `Calculator` or `Calculator<T>`)
                /// - `$interface_name`: The interface being implemented (e.g., `ICalculator`)
                /// - `$vtable_field`: The vtable pointer field name (e.g., `vtable_i_calculator`)
                /// - `$iid_const`: The IID constant for the interface (unused but kept for consistency)
                #[macro_export]
                macro_rules! #forwarders_macro_name {
                    ($struct_name:ident, $struct_type:ty, $interface_name:ident, $vtable_field:ident, $iid_const:ident) => {
                        $crate::paste! {
                            #(#forwarder_wrappers)*
                        }
                    };
                }
            },
            quote! {
                /// Auto-generated base vtable initializer macro for #trait_name.
                ///
                /// Returns an expression that creates `#vtable_name { ... }` with the wrapper function pointers.
                #[macro_export]
                macro_rules! #base_vtable_macro_name {
                    ($struct_name:ident, $interface_name:ident) => {
                        $crate::paste! {
                            #vtable_name {
                                #(#vtable_entries),*
                            }
                        }
                    };
                }
            },
        )
    };

    // Generate PhantomData field for generic interfaces to avoid unused type parameter errors
    let phantom_field = if has_type_params {
        quote! { _phantom: std::marker::PhantomData #type_generics, }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #iid_definition

        #vtable_struct

        /// Base struct representing the interface pointer
        #[repr(C)]
        #vis struct #trait_name #impl_generics #where_clause {
            vtable: *const #vtable_name #type_generics,
            #phantom_field
        }

        impl #impl_generics #trait_name #type_generics #where_clause {
            #iid_methods

            /// Get the vtable
            #[inline]
            #[must_use]
            pub fn vtable(&self) -> &#vtable_name #type_generics {
                unsafe { &*self.vtable }
            }

            /// Wrap a raw pointer for calling methods.
            ///
            /// # Safety
            ///
            /// - `ptr` must point to a valid object with a compatible vtable layout
            /// - The returned reference must not outlive the underlying object
            /// - The caller is responsible for ensuring the lifetime `'a` is valid
            /// - No mutable references to the same object may exist concurrently
            #[inline]
            pub unsafe fn from_ptr<'a>(ptr: #self_ptr_type) -> &'a Self {
                std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
                let ptr = std::ptr::read_volatile(&ptr);
                &*(ptr as *const Self)
            }

            /// Wrap a raw pointer for calling methods (mutable).
            ///
            /// # Safety
            ///
            /// - `ptr` must point to a valid object with a compatible vtable layout
            /// - The returned reference must not outlive the underlying object
            /// - The caller is responsible for ensuring the lifetime `'a` is valid
            /// - No other references to the same object may exist concurrently
            #[inline]
            pub unsafe fn from_ptr_mut<'a>(ptr: #self_ptr_type) -> &'a mut Self {
                std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
                let ptr = std::ptr::read_volatile(&ptr);
                &mut *(ptr as *mut Self)
            }

            #iunknown_wrappers

            #(#wrapper_methods)*
        }

        impl #impl_generics #krate::VTableLayout for #trait_name #type_generics #where_clause {
            const SLOT_COUNT: usize = #slot_count_expr;
            type VTable = #vtable_name #type_generics;
        }

        #forwarders_macro
        #base_vtable_macro
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
/// # Options
/// - `stdcall` - Use stdcall calling convention on x86 (default: thiscall)
/// - `extends(IUnknown)` - Inherit IUnknown methods at slots 0-2
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
///
/// // COM-style with stdcall and IUnknown base
/// #[cppvtable(stdcall, extends(IUnknown))]
/// pub trait IComStyle {
///     fn method(&self) -> i32;   // slot 3 (after IUnknown)
/// }
/// ```
#[proc_macro_attribute]
pub fn cppvtable(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemTrait);

    // Parse configuration from attributes
    let config = match parse_cppvtable_config(attr) {
        Ok(config) => config,
        Err(err) => return err.to_compile_error().into(),
    };

    match cppvtable_internal(config, input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Parse cppvtable attribute options into a VTableConfig
fn parse_cppvtable_config(attr: TokenStream) -> Result<VTableConfig, syn::Error> {
    let mut config = VTableConfig::default();

    if attr.is_empty() {
        return Ok(config);
    }

    // Parse the attribute as a comma-separated list
    let attr2: TokenStream2 = attr.into();
    let tokens: Vec<_> = attr2.into_iter().collect();

    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            proc_macro2::TokenTree::Ident(ident) => {
                let name = ident.to_string();
                match name.as_str() {
                    "stdcall" => {
                        config.calling_convention = CallingConvention::Stdcall;
                        i += 1;
                    }
                    "thiscall" => {
                        config.calling_convention = CallingConvention::Thiscall;
                        i += 1;
                    }
                    "extends" => {
                        // Expect: extends(BaseInterface)
                        i += 1;
                        if i >= tokens.len() {
                            return Err(syn::Error::new(
                                ident.span(),
                                "expected '(' after 'extends'",
                            ));
                        }
                        if let proc_macro2::TokenTree::Group(group) = &tokens[i] {
                            // Parse the base interface identifier
                            let base_ident: syn::Ident =
                                syn::parse2(group.stream()).map_err(|_| {
                                    syn::Error::new(
                                        group.span(),
                                        "expected an identifier inside 'extends(...)'",
                                    )
                                })?;
                            config.base_interface = Some(base_ident);
                            i += 1;
                        } else {
                            return Err(syn::Error::new(
                                ident.span(),
                                "expected '(...)' after 'extends'",
                            ));
                        }
                    }
                    "slots" => {
                        // Expect: slots(method = N, ...)
                        i += 1;
                        if i >= tokens.len() {
                            return Err(syn::Error::new(
                                ident.span(),
                                "expected '(' after 'slots'",
                            ));
                        }
                        if let proc_macro2::TokenTree::Group(group) = &tokens[i] {
                            config.slot_overrides =
                                parse_slot_overrides_from_stream(group.stream());
                            i += 1;
                        } else {
                            return Err(syn::Error::new(
                                ident.span(),
                                "expected '(...)' after 'slots'",
                            ));
                        }
                    }
                    "no_iid" => {
                        // Skip IID generation - user defines their own IID
                        config.iid = InterfaceId::None;
                        i += 1;
                    }
                    "internal" => {
                        // Use crate:: instead of cppvtable:: for paths
                        // This is used when defining interfaces inside the cppvtable crate itself
                        config.internal = true;
                        i += 1;
                    }
                    "no_forwarders" => {
                        // Skip generating forwarder macros
                        // Use when forwarders need to be manually defined (e.g., IUnknown with COM types)
                        config.no_forwarders = true;
                        i += 1;
                    }
                    _ => {
                        return Err(syn::Error::new(
                            ident.span(),
                            format!(
                                "unknown option '{}', expected 'stdcall', 'thiscall', 'extends(...)', 'slots(...)', 'no_iid', 'internal', or 'no_forwarders'",
                                name
                            ),
                        ));
                    }
                }
            }
            proc_macro2::TokenTree::Punct(punct) if punct.as_char() == ',' => {
                i += 1; // Skip commas
            }
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    "unexpected token in cppvtable options",
                ));
            }
        }
    }

    Ok(config)
}

/// Parse slot overrides from a token stream: method = N, ...
fn parse_slot_overrides_from_stream(
    stream: TokenStream2,
) -> std::collections::HashMap<String, usize> {
    let mut result = std::collections::HashMap::new();
    let tokens: Vec<_> = stream.into_iter().collect();

    /// Extract a literal value from a TokenTree, handling nested Groups
    fn extract_literal(tt: &proc_macro2::TokenTree) -> Option<String> {
        match tt {
            proc_macro2::TokenTree::Literal(lit) => Some(lit.to_string()),
            proc_macro2::TokenTree::Group(g) => {
                // Handle case where macro expansion wraps literal in a None-delimited group
                let inner: Vec<_> = g.stream().into_iter().collect();
                if inner.len() == 1 {
                    extract_literal(&inner[0])
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    let mut i = 0;
    // Need at least 3 tokens: name = value
    while i + 3 <= tokens.len() {
        // Check for Ident = Value pattern
        if let proc_macro2::TokenTree::Ident(name) = &tokens[i]
            && let proc_macro2::TokenTree::Punct(eq) = &tokens[i + 1]
            && eq.as_char() == '='
        {
            // Try to extract the value (may be Literal or Group containing Literal)
            if let Some(lit_str) = extract_literal(&tokens[i + 2])
                && let Ok(slot) = lit_str.parse::<usize>()
            {
                result.insert(name.to_string(), slot);
            }
            i += 3;
            // Skip comma if present
            if i < tokens.len()
                && let proc_macro2::TokenTree::Punct(p) = &tokens[i]
                && p.as_char() == ','
            {
                i += 1;
            }
            continue;
        }
        i += 1;
    }

    result
}

/// Internal implementation of cppvtable_impl
fn cppvtable_impl_impl(interface_name: Ident, input: ItemImpl) -> Result<TokenStream2, syn::Error> {
    // Use default config for regular C++ vtables
    let config = ImplConfig {
        calling_convention: CallingConvention::Thiscall,
        base_interface: None,
        first_slot: 0,
        generate_rtti: true,
        iid_const: None,
        internal: false,
    };
    cppvtable_impl_internal(interface_name, input, config)
}

/// Core implementation shared by cppvtable_impl and com_implement
fn cppvtable_impl_internal(
    interface_name: Ident,
    input: ItemImpl,
    config: ImplConfig,
) -> Result<TokenStream2, syn::Error> {
    // Validate impl block for C++ vtable compatibility
    validate_impl(&input)?;

    let struct_type = &input.self_ty;
    let vtable_name = format_ident!("{}VTable", interface_name);

    // Extract struct name for generating identifiers
    let struct_name = match struct_type.as_ref() {
        Type::Path(type_path) => type_path.path.segments.last().unwrap().ident.clone(),
        _ => return Err(syn::Error::new(struct_type.span(), "Expected a type path")),
    };

    // Derive vtable field name from interface name
    let vtable_field = interface_to_field_name(&interface_name);

    // x86 calling convention
    let x86_cc = config.x86_calling_conv();

    // Collect methods with their slot indices
    struct ImplMethodInfo {
        slot: usize,
        name: Ident,
        param_names: Vec<Ident>,
        param_types: Vec<Type>,
        output: syn::ReturnType,
        is_mut: bool,
        original: syn::ImplItemFn,
    }

    let mut methods: Vec<ImplMethodInfo> = Vec::new();
    let mut next_slot = config.first_slot;

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
                    if let FnArg::Typed(pat_type) = arg
                        && let Pat::Ident(pat_ident) = pat_type.pat.as_ref()
                    {
                        let name = &pat_ident.ident;
                        let ty = pat_type.ty.as_ref();
                        return Some((name.clone(), ty.clone()));
                    }
                    None
                })
                .collect();

            // Check if method takes &self or &mut self
            let is_mut = method
                .sig
                .inputs
                .first()
                .is_some_and(|arg| matches!(arg, FnArg::Receiver(r) if r.mutability.is_some()));

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

    // Generate wrapper functions and vtable entries, filling gaps
    let mut wrapper_fns = Vec::new();
    let mut vtable_entries = Vec::new();
    let mut original_methods = Vec::new();
    let mut current_slot = config.first_slot;

    // Generate base interface forwarders, vtable entry, and methods if configured
    // Uses convention: base interface `IFoo` provides macros `ifoo_forwarders!`, `ifoo_base_vtable!`, `ifoo_methods!`
    let krate = crate_path(config.internal);
    let (base_forwarders, base_vtable_entry, base_methods) = if let Some(base) =
        &config.base_interface
    {
        let base_lower = base.to_string().to_lowercase();
        let forwarders_macro = format_ident!("{}_forwarders", base_lower);
        let base_vtable_macro = format_ident!("{}_base_vtable", base_lower);
        let methods_macro = format_ident!("{}_methods", base_lower);

        let iid_const =
            config.iid_const.as_ref().cloned().unwrap_or_else(|| {
                format_ident!("IID_{}", interface_name.to_string().to_uppercase())
            });

        let forwarders = quote! {
            #krate::#forwarders_macro!(#struct_name, #struct_type, #interface_name, #vtable_field, #iid_const);
        };

        let vtable_entry = quote! {
            base: #krate::#base_vtable_macro!(#struct_name, #interface_name)
        };

        let methods = quote! {
            #krate::#methods_macro!(#struct_type, #vtable_field, #iid_const);
        };

        (Some(forwarders), Some(vtable_entry), Some(methods))
    } else {
        (None, None, None)
    };

    for method in &methods {
        // Fill gaps with dummy panic stubs (only for slots after first_slot)
        while current_slot < method.slot {
            let dummy_name = format_ident!("__reserved_slot_{}", current_slot);
            let dummy_wrapper =
                format_ident!("__{}__{}__{}", struct_name, interface_name, dummy_name);

            wrapper_fns.push(quote! {
                #[allow(non_snake_case)]
                #[cfg(target_arch = "x86")]
                unsafe extern #x86_cc fn #dummy_wrapper(_this: *mut std::ffi::c_void) {
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
        // x86: thiscall/stdcall depending on config, x64: C calling convention
        wrapper_fns.push(quote! {
            #[allow(non_snake_case)]
            #[cfg(target_arch = "x86")]
            unsafe extern #x86_cc fn #wrapper_name(
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

    // Build vtable entries with optional base vtable entry (e.g., base: IUnknownVTable { ... })
    let vtable_body = if let Some(base_entry) = &base_vtable_entry {
        quote! {
            #base_entry,
            #(#vtable_entries),*
        }
    } else {
        quote! {
            #(#vtable_entries),*
        }
    };

    // Generate RTTI if configured
    let rtti_const = if config.generate_rtti {
        let interface_info_const_name = format_ident!(
            "INTERFACE_INFO_{}",
            vtable_field
                .to_string()
                .trim_start_matches("vtable_")
                .to_uppercase()
        );
        quote! {
            /// RTTI: Interface info for this interface implementation.
            /// Contains interface ID and byte offset from struct start.
            pub const #interface_info_const_name: #krate::InterfaceInfo = #krate::InterfaceInfo {
                interface_id: #interface_name::interface_id_ptr(),
                offset: ::std::mem::offset_of!(Self, #vtable_field) as isize,
            };
        }
    } else {
        quote! {}
    };

    // Generate IID const for COM interfaces
    let iid_const = if let Some(iid_name) = &config.iid_const {
        quote! {
            /// COM IID for this interface
            pub const IID: &'static #krate::GUID = &#iid_name;
        }
    } else {
        quote! {}
    };

    // Extra methods from base interface (e.g., query_interface/add_ref/release for IUnknown)
    let extra_methods = base_methods.unwrap_or_default();

    let expanded = quote! {
        // Base interface forwarders (e.g., IUnknown wrapper functions)
        #base_forwarders

        // The wrapper functions (private)
        #(#wrapper_fns)*

        // Static vtable instance
        static #vtable_static_name: #vtable_name = #vtable_name {
            #vtable_body
        };

        // Original impl with methods + vtable const accessor
        impl #struct_type {
            /// Pointer to the vtable for this interface implementation.
            /// Use this when constructing the struct.
            pub const #vtable_const_name: *const #vtable_name = &#vtable_static_name;

            #iid_const
            #rtti_const

            #(#original_methods)*

            #extra_methods
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

// =============================================================================
// COM Interface Support
// =============================================================================

/// Parse a GUID string in format "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
/// Returns (data1, data2, data3, data4) tuple
fn parse_guid_string(s: &str) -> Result<(u32, u16, u16, [u8; 8]), String> {
    let s = s.trim();
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return Err(format!(
            "Invalid GUID format: expected 'xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx', got '{}'",
            s
        ));
    }

    let data1 = u32::from_str_radix(parts[0], 16)
        .map_err(|_| format!("Invalid GUID data1: '{}'", parts[0]))?;
    let data2 = u16::from_str_radix(parts[1], 16)
        .map_err(|_| format!("Invalid GUID data2: '{}'", parts[1]))?;
    let data3 = u16::from_str_radix(parts[2], 16)
        .map_err(|_| format!("Invalid GUID data3: '{}'", parts[2]))?;

    // parts[3] is 4 hex chars (2 bytes), parts[4] is 12 hex chars (6 bytes)
    if parts[3].len() != 4 {
        return Err(format!(
            "Invalid GUID data4 first part: expected 4 hex chars, got '{}'",
            parts[3]
        ));
    }
    if parts[4].len() != 12 {
        return Err(format!(
            "Invalid GUID data4 second part: expected 12 hex chars, got '{}'",
            parts[4]
        ));
    }

    let mut data4 = [0u8; 8];
    data4[0] = u8::from_str_radix(&parts[3][0..2], 16)
        .map_err(|_| format!("Invalid GUID data4[0]: '{}'", &parts[3][0..2]))?;
    data4[1] = u8::from_str_radix(&parts[3][2..4], 16)
        .map_err(|_| format!("Invalid GUID data4[1]: '{}'", &parts[3][2..4]))?;
    for i in 0..6 {
        data4[2 + i] = u8::from_str_radix(&parts[4][i * 2..i * 2 + 2], 16).map_err(|_| {
            format!(
                "Invalid GUID data4[{}]: '{}'",
                2 + i,
                &parts[4][i * 2..i * 2 + 2]
            )
        })?;
    }

    Ok((data1, data2, data3, data4))
}

/// Define a COM interface.
///
/// This generates:
/// - A vtable struct `{Name}VTable` with IUnknown methods (slots 0-2) + your methods
/// - An interface wrapper struct `{Name}` with method wrappers
/// - An IID constant `IID_{NAME}` parsed from the GUID string
///
/// Uses `stdcall` calling convention on x86 (not `thiscall` like C++ vtables).
///
/// # Example
/// ```ignore
/// #[com_interface("12345678-1234-1234-1234-123456789abc")]
/// pub trait IMyInterface {
///     fn do_something(&self, x: i32) -> HRESULT;
///     #[slot(5)]
///     fn do_other(&self) -> HRESULT;  // slot 5 (slots 3-4 filled with dummies)
/// }
/// ```
#[proc_macro_attribute]
pub fn com_interface(attr: TokenStream, item: TokenStream) -> TokenStream {
    // Parse the GUID string from the attribute
    let guid_str: syn::LitStr = match syn::parse(attr) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error().into(),
    };

    // Parse GUID
    let (data1, data2, data3, data4) = match parse_guid_string(&guid_str.value()) {
        Ok(parsed) => parsed,
        Err(e) => {
            return syn::Error::new(guid_str.span(), e)
                .to_compile_error()
                .into();
        }
    };

    // Create COM config: stdcall + extends(IUnknown) + GUID IID
    let config = VTableConfig {
        calling_convention: CallingConvention::Stdcall,
        base_interface: Some(syn::Ident::new("IUnknown", proc_macro2::Span::call_site())),
        iid: InterfaceId::Guid {
            data1,
            data2,
            data3,
            data4,
        },
        slot_overrides: std::collections::HashMap::new(),
        internal: false,
        no_forwarders: false,
    };

    let input = parse_macro_input!(item as ItemTrait);
    match cppvtable_internal(config, input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Internal implementation of com_implement
fn com_implement_internal(
    interface_name: Ident,
    input: ItemImpl,
) -> Result<TokenStream2, syn::Error> {
    // COM uses stdcall, inherits from IUnknown (3 slots), no RTTI
    let iid_const = format_ident!("IID_{}", interface_name.to_string().to_uppercase());

    let config = ImplConfig {
        calling_convention: CallingConvention::Stdcall,
        base_interface: Some(format_ident!("IUnknown")),
        first_slot: 3, // IUnknown has QueryInterface, AddRef, Release
        generate_rtti: false,
        iid_const: Some(iid_const),
        internal: false,
    };

    cppvtable_impl_internal(interface_name, input, config)
}

/// Implement a COM interface for a struct.
///
/// This generates:
/// - Static vtable instance with IUnknown methods (QueryInterface, AddRef, Release)
/// - Wrapper functions that cast `this` and call your methods
/// - A vtable accessor constant (`VTABLE_I_INTERFACE_NAME`)
/// - IUnknown methods on the struct (`query_interface`, `add_ref`, `release`)
///
/// # Requirements
///
/// Your struct must have:
/// - A `ref_count: ComRefCount` field for reference counting
/// - A vtable pointer field named `vtable_i_{interface_name}` (auto-derived from interface name)
///
/// # Example
/// ```ignore
/// #[repr(C)]
/// struct MyObject {
///     vtable_i_my_interface: *const IMyInterfaceVTable,
///     ref_count: ComRefCount,
///     // ... other fields
/// }
///
/// impl MyObject {
///     pub fn new() -> Self {
///         Self {
///             vtable_i_my_interface: Self::VTABLE_I_MY_INTERFACE,
///             ref_count: ComRefCount::new(),
///         }
///     }
/// }
///
/// #[com_implement(IMyInterface)]
/// impl MyObject {
///     fn do_something(&self, x: i32) -> HRESULT { S_OK }
/// }
/// ```
#[proc_macro_attribute]
pub fn com_implement(attr: TokenStream, item: TokenStream) -> TokenStream {
    let interface_name = parse_macro_input!(attr as Ident);
    let input = parse_macro_input!(item as ItemImpl);
    match com_implement_internal(interface_name, input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
