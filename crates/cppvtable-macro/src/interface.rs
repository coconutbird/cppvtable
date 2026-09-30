//! The code generator of the shared `#[interface]` macro.
//!
//! The ABI entry point emits only the interface wrapper, vtable, and metadata needed to
//! call a foreign object. The C/C++ and COM runtime entry points also emit implementation
//! shims, an implementer trait, and a vtable builder for their respective object models.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, ItemTrait, Path, Visibility};

use crate::abi::{Abi, AbiVariant};
use crate::parse::{
    InterfaceArgs, InterfaceModel, Layout, Method, Runtime, derived_name, derived_path, shim_name,
};
use crate::validate::{ReturnKind, needs_non_snake_case};

#[cfg(test)]
mod tests;

/// The largest number of arguments that `clippy::too_many_arguments` permits.
const ARGUMENT_LIMIT: usize = 7;

/// The base of an interface chain.
enum Base {
    /// The interface has no base. Only `IUnknown` and a root `cpp` or `c` interface use
    /// this.
    None,
    /// The base is `IUnknown`. Each COM interface without `extends` uses this.
    Unknown,
    /// The base is another declared interface.
    Interface(Path),
}

/// Expand the full interface macro used by the COM runtime crate.
pub(crate) fn expand(args: TokenStream, item: &ItemTrait) -> Result<TokenStream, syn::Error> {
    let args = InterfaceArgs::parse(args)?;
    if !args.abi.is_com() {
        return Err(syn::Error::new_spanned(
            item,
            "use cppvtable::interface for C/C++ implementations",
        ));
    }
    let model = InterfaceModel::parse(item)?;
    configure(generate(&args, &model, Runtime::Com, true), &model.cfgs)
}

/// Expand the standalone ABI crate's foreign interface macro.
pub(crate) fn expand_abi(args: TokenStream, item: &ItemTrait) -> Result<TokenStream, syn::Error> {
    expand_abi_with_runtime(args, item, Runtime::Abi)
}

/// Expand the interface macro re-exported by the ordinary C/C++ runtime crate.
pub(crate) fn expand_native(
    args: TokenStream,
    item: &ItemTrait,
) -> Result<TokenStream, syn::Error> {
    let args = InterfaceArgs::parse(args)?;
    if args.abi.is_com() {
        return Err(syn::Error::new_spanned(
            item,
            "use cppvtable_com::interface for COM interfaces",
        ));
    }
    let model = InterfaceModel::parse(item)?;
    configure(generate(&args, &model, Runtime::Native, true), &model.cfgs)
}

fn expand_abi_with_runtime(
    args: TokenStream,
    item: &ItemTrait,
    runtime: Runtime,
) -> Result<TokenStream, syn::Error> {
    let args = InterfaceArgs::parse(args)?;
    if args.abi.is_com() {
        return Err(syn::Error::new_spanned(
            item,
            "the ABI crate declares only `abi = cpp`, `msvc`, `itanium`, or `c` interfaces",
        ));
    }
    let model = InterfaceModel::parse(item)?;
    configure(generate(&args, &model, runtime, false), &model.cfgs)
}

/// Put the declaration's `#[cfg]` attributes on every generated item, so a false
/// predicate removes all of them together.
fn configure(tokens: TokenStream, cfgs: &[syn::Attribute]) -> Result<TokenStream, syn::Error> {
    if cfgs.is_empty() {
        return Ok(tokens);
    }
    let file: syn::File = syn::parse2(tokens)?;
    let items = file.items.iter().map(|item| quote! { #(#cfgs)* #item });
    Ok(quote! { #(#items)* })
}

/// Make the whole output of the macro.
fn generate(
    args: &InterfaceArgs,
    model: &InterfaceModel,
    runtime: Runtime,
    implementable: bool,
) -> TokenStream {
    let krate = args.krate(runtime);
    let abi_crate = if args.internal {
        krate.clone()
    } else {
        InterfaceArgs::abi_krate(runtime)
    };
    let name = &model.name;
    let vis = &model.vis;
    let vtbl_name = derived_name(name, "Vtbl");
    let impl_name = derived_name(name, "Impl");
    let base = base_of(args);
    let trailing_slots = args.slots.map(|total| {
        crate::layout::trailing_slots(total, model.slots.len(), base.vtbl_type(&krate))
    });
    let is_plain = matches!(runtime, Runtime::Native);
    let object = if is_plain {
        quote! { #krate::Object }
    } else {
        quote! { #krate::ComObject }
    };
    let implementation = if is_plain {
        quote! { #krate::Implement }
    } else {
        quote! { #krate::ComImplement }
    };
    let generate_shims = implementable && (!args.root || is_plain);
    let variants = args.abi.variants(model.uses_x86_conventions());
    let hook = (is_plain && args.layout == Layout::Pointer).then(|| hook_method(&krate, vis));

    let mut output = args.abi.target_guard();
    for variant in &variants {
        output.extend(vtable_struct(
            model,
            variant,
            &base,
            &krate,
            &vtbl_name,
            trailing_slots.as_ref(),
        ));
        if generate_shims {
            output.extend(shims(model, variant, &object, &impl_name));
        }
    }
    output.extend(interface_type(
        model,
        args.layout,
        &abi_crate,
        &vtbl_name,
        &variants,
        hook.as_ref(),
    ));
    output.extend(interface_trait_impl(
        args, model, &base, &krate, &abi_crate, &vtbl_name, is_plain,
    ));
    output.extend(deref_to_base(model, &base, &krate));
    if generate_shims {
        output.extend(impl_trait(model, &base, &implementation, &impl_name));
        output.extend(vtable_builder(
            model,
            &base,
            &krate,
            &vtbl_name,
            &impl_name,
            vis,
            trailing_slots.as_ref(),
        ));
    }
    output
}

/// Decide the base of the chain.
fn base_of(args: &InterfaceArgs) -> Base {
    if args.root {
        Base::None
    } else if let Some(path) = &args.extends {
        Base::Interface(path.clone())
    } else if args.abi.is_com() {
        Base::Unknown
    } else {
        Base::None
    }
}

impl Base {
    /// Give the type of the vtable of the base.
    fn vtbl_type(&self, krate: &TokenStream) -> Option<TokenStream> {
        match self {
            Self::None => None,
            Self::Unknown => Some(quote! { #krate::IUnknownVtbl }),
            Self::Interface(path) => {
                let vtbl = derived_path(path, "Vtbl");
                Some(quote! { #vtbl })
            }
        }
    }

    /// Give the interface type of the base.
    fn interface_type(&self, krate: &TokenStream) -> Option<TokenStream> {
        match self {
            Self::None => None,
            Self::Unknown => Some(quote! { #krate::IUnknown }),
            Self::Interface(path) => Some(quote! { #path }),
        }
    }

    /// Give the supertrait of the `Impl` trait.
    fn impl_bound(&self, implementation: &TokenStream) -> TokenStream {
        match self {
            Self::None | Self::Unknown => quote! { #implementation },
            Self::Interface(path) => {
                let bound = derived_path(path, "Impl");
                quote! { #bound }
            }
        }
    }
}

/// Make the `#[repr(C)]` vtable structure.
fn vtable_struct(
    model: &InterfaceModel,
    variant: &AbiVariant,
    base: &Base,
    krate: &TokenStream,
    vtbl_name: &Ident,
    trailing_slots: Option<&TokenStream>,
) -> TokenStream {
    let vis = &model.vis;
    let name = &model.name;
    let cfg = variant.cfg();
    let convention = variant.convention;
    let trailing_field = trailing_slots.map(|count| {
        quote! {
            /// Unknown trailing entries included in the declared total vtable size.
            pub __reserved_tail: [::core::option::Option<unsafe extern #convention fn()>; #count],
        }
    });
    let doc = format!("The vtable of [`{name}`]. The layout is the layout of the C++ vtable.");
    let base_field = base.vtbl_type(krate).map(|ty| {
        quote! {
            /// The vtable of the base interface. It must be the first field.
            pub base: #ty,
        }
    });

    let fields = model.slots.iter().map(|slot| {
        if let Some(method) = &slot.method {
            let docs = &method.docs;
            let field = &method.name;
            let ty = pointer_type(method, variant);
            quote! {
                #(#docs)*
                pub #field: #ty,
            }
        } else {
            let field = format_ident!("reserved_{}", slot.index);
            let doc = format!(
                "A reserved entry. `#[slot(N)]` made it to keep the slot numbers of \
                 the foreign header. Slot {} of this interface.",
                slot.index
            );
            quote! {
                #[doc = #doc]
                pub #field: ::core::option::Option<unsafe extern #convention fn()>,
            }
        }
    });

    let expect = non_snake_case_expect(
        &model.method_names(),
        "The field names are the method names of the foreign header.",
    );
    quote! {
        #[doc = #doc]
        #cfg
        #expect
        #[repr(C)]
        #[derive(Clone, Copy)]
        #vis struct #vtbl_name {
            #base_field
            #(#fields)*
            #trailing_field
        }
    }
}

/// Choose the target-specific lowering of a portable aggregate return.
fn effective_kind(kind: ReturnKind, variant: &AbiVariant) -> ReturnKind {
    if kind == ReturnKind::Aggregate && variant.aggregate_hidden {
        ReturnKind::Hidden
    } else {
        kind
    }
}

/// The leading arguments of an explicitly lowered indirect return.
fn hidden_parameters(variant: &AbiVariant, ret: &TokenStream) -> TokenStream {
    if variant.hidden_before_this {
        quote! { result: *mut #ret, this: *mut ::core::ffi::c_void, }
    } else {
        quote! { this: *mut ::core::ffi::c_void, result: *mut #ret, }
    }
}

/// Use the method override consistently for its vtable field, shim, and caller.
///
/// An x86-only override lowers to `"C"` in the versions for other architectures.
fn method_convention(method: &Method, variant: &AbiVariant) -> syn::LitStr {
    variant.convention_for(method.convention.as_ref())
}

/// Give the type of the function pointer of a method.
fn pointer_type(method: &Method, variant: &AbiVariant) -> TokenStream {
    let names = method.params.iter().map(|param| &param.name);
    let types = method.params.iter().map(|param| &param.ty);
    let convention = method_convention(method, variant);
    match effective_kind(method.kind, variant) {
        ReturnKind::Hidden => {
            let ret = return_type(method);
            let prefix = hidden_parameters(variant, &ret);
            quote! {
                unsafe extern #convention fn(
                    #prefix
                    #(#names: #types),*
                ) -> *mut #ret
            }
        }
        ReturnKind::Scalar | ReturnKind::Unit | ReturnKind::Aggregate => {
            let output = &method.output;
            quote! {
                unsafe extern #convention fn(
                    this: *mut ::core::ffi::c_void,
                    #(#names: #types),*
                ) #output
            }
        }
    }
}

/// Give the declared return type of a method that has one.
fn return_type(method: &Method) -> TokenStream {
    match &method.output {
        syn::ReturnType::Type(_, ty) => quote! { #ty },
        syn::ReturnType::Default => quote! { () },
    }
}

/// Make the shim of each method.
fn shims(
    model: &InterfaceModel,
    variant: &AbiVariant,
    object: &TokenStream,
    impl_name: &Ident,
) -> TokenStream {
    let cfg = variant.cfg();
    let name = &model.name;
    let items = model.slots.iter().filter_map(|slot| {
        let method = slot.method.as_ref()?;
        let convention = method_convention(method, variant);
        let shim = shim_name(name, slot.index);
        let method_name = &method.name;
        let names: Vec<&Ident> = method.params.iter().map(|param| &param.name).collect();
        let types = method.params.iter().map(|param| &param.ty);
        let doc = format!("The shim of [`{name}::{method_name}`].");
        let allow = method.deprecated.then(|| {
            quote! {
                #[allow(
                    deprecated,
                    reason = "The shim calls the implementation of a deprecated method."
                )]
            }
        });
        Some(match effective_kind(method.kind, variant) {
            ReturnKind::Hidden => {
                let ret = return_type(method);
                let expect = many_arguments_expect(method.params.len() + 2);
                let prefix = hidden_parameters(variant, &ret);
                quote! {
                    #[doc = #doc]
                    #cfg
                    #expect
                    #allow
                    unsafe extern #convention fn #shim<T: #impl_name, const SLOT: usize>(
                        #prefix
                        #(#names: #types),*
                    ) -> *mut #ret {
                        unsafe {
                            let object = #object::<T>::impl_from_slot(this, SLOT);
                            let value = <T as #impl_name>::#method_name(object #(, #names)*);
                            ::core::ptr::write(result, value);
                            result
                        }
                    }
                }
            }
            ReturnKind::Scalar | ReturnKind::Unit | ReturnKind::Aggregate => {
                let output = &method.output;
                let expect = many_arguments_expect(method.params.len() + 1);
                quote! {
                    #[doc = #doc]
                    #cfg
                    #expect
                    #allow
                    unsafe extern #convention fn #shim<T: #impl_name, const SLOT: usize>(
                        this: *mut ::core::ffi::c_void,
                        #(#names: #types),*
                    ) #output {
                        unsafe {
                            let object = #object::<T>::impl_from_slot(this, SLOT);
                            <T as #impl_name>::#method_name(object #(, #names)*)
                        }
                    }
                }
            }
        })
    });
    quote! { #(#items)* }
}

/// Give `#[allow(deprecated)]` for the generated items that name a deprecated interface.
fn allow_deprecated(model: &InterfaceModel) -> Option<TokenStream> {
    model.deprecated.then(|| {
        quote! {
            #[allow(
                deprecated,
                reason = "The generated items implement the deprecated interface."
            )]
        }
    })
}

/// Make the interface type and its methods.
fn interface_type(
    model: &InterfaceModel,
    layout: Layout,
    abi_crate: &TokenStream,
    vtbl_name: &Ident,
    variants: &[AbiVariant],
    hook: Option<&TokenStream>,
) -> TokenStream {
    let name = &model.name;
    let vis = &model.vis;
    let docs = &model.docs;
    let attrs = &model.attrs;
    let allow = allow_deprecated(model);
    let type_doc = format!(
        "The interface `{name}`.\n\n\
         The type is a transparent wrapper of one interface pointer. Borrow a foreign \
         pointer with [`{name}::from_raw`] or [`{name}::from_non_null`]; the caller keeps \
         the object alive for the borrow. A value of this type exists only behind a borrow \
         and cannot be copied out of it, so it never outlives its object.\n\n\
         Methods declared `fn` are safe to call. The `unsafe trait` declaration is the \
         proof: it promises that the slot order, signatures, calling conventions, and \
         return lowering match the foreign header, that every method declared as a safe \
         `fn` has no precondition beyond a live object, and that no method unwinds. \
         Methods declared `unsafe fn` keep the preconditions that they document.\n\n\
         The debug output is `{name}(0x…)`, and equality compares the interface pointers."
    );
    let debug_format = format!("{name}({{:p}})");
    let vtable_body = match layout {
        Layout::Pointer => quote! {
            unsafe { &**self.as_raw().cast::<*const #vtbl_name>() }
        },
        Layout::Inline => quote! {
            unsafe { &*self.as_raw().cast::<#vtbl_name>() }
        },
    };

    let methods = variants.iter().flat_map(|variant| {
        model.slots.iter().filter_map(move |slot| {
            let method = slot.method.as_ref()?;
            let cfg = variant.cfg();
            let caller = caller_method(method, vis, variant);
            Some(quote! { #cfg #caller })
        })
    });

    let expect = non_snake_case_expect(
        &model.method_names(),
        "The method names are the names of the foreign header.",
    );

    quote! {
        #(#docs)*
        #[doc = ""]
        #[doc = #type_doc]
        #(#attrs)*
        #[repr(transparent)]
        #vis struct #name(#abi_crate::RawInterface);

        #allow
        const _: () = assert!(
            ::core::mem::size_of::<#name>() == ::core::mem::size_of::<*mut ::core::ffi::c_void>()
                && ::core::mem::size_of::<
                    ::core::option::Option<#abi_crate::InterfaceRef<'static, #name>>,
                >() == ::core::mem::size_of::<*mut ::core::ffi::c_void>(),
            "an interface and an optional interface reference must be one pointer"
        );

        #allow
        #expect
        impl #name {
            /// Give the raw interface pointer.
            #[inline]
            #[must_use]
            #vis const fn as_raw(&self) -> *mut ::core::ffi::c_void {
                self.0.as_ptr()
            }

            /// Borrow a raw interface pointer. A null pointer gives `None`.
            ///
            /// # Safety
            ///
            /// A non-null `raw` must be a valid interface pointer of this interface for
            /// the whole lifetime `'a`: the object must stay alive, its function table
            /// must stay valid and unmodified, and the object must implement this
            /// declaration.
            #[inline]
            #[must_use]
            #vis unsafe fn from_raw<'a>(
                raw: *mut ::core::ffi::c_void,
            ) -> ::core::option::Option<#abi_crate::InterfaceRef<'a, Self>> {
                unsafe { #abi_crate::InterfaceRef::from_raw(raw) }
            }

            /// Borrow a non-null raw interface pointer.
            ///
            /// # Safety
            ///
            /// `raw` must be a valid interface pointer of this interface for the whole
            /// lifetime `'a`: the object must stay alive, its function table must stay
            /// valid and unmodified, and the object must implement this declaration.
            #[inline]
            #[must_use]
            #vis unsafe fn from_non_null<'a>(
                raw: ::core::ptr::NonNull<::core::ffi::c_void>,
            ) -> #abi_crate::InterfaceRef<'a, Self> {
                unsafe { #abi_crate::InterfaceRef::from_non_null(raw) }
            }

            /// Give the function table of the object, borrowed as long as `self`.
            #[inline]
            #[must_use]
            #vis fn vtable(&self) -> &#vtbl_name {
                #vtable_body
            }

            #hook

            #(#methods)*
        }

        #allow
        impl ::core::fmt::Debug for #name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                ::core::write!(f, #debug_format, self.as_raw())
            }
        }

        #allow
        impl ::core::cmp::PartialEq for #name {
            #[inline]
            fn eq(&self, other: &Self) -> bool {
                ::core::ptr::eq(self.as_raw(), other.as_raw())
            }
        }

        #allow
        impl ::core::cmp::Eq for #name {}
    }
}

/// Make the `hook` method of a native pointer-layout interface.
fn hook_method(krate: &TokenStream, vis: &Visibility) -> TokenStream {
    quote! {
        /// Hook the vtable of this object with `cppvtable::hook::VtableHook::new`.
        ///
        /// Edit the active table with `VtableHook::set`, `hook`, and `unhook`. Dropping
        /// the hook restores the original table.
        ///
        /// # Safety
        ///
        /// The contract of `VtableHook::new` applies. In short: the object must stay
        /// live with no concurrent access to its vtable pointer until the hook drops; the
        /// ordinary RTTI prefix of this interface's C++ ABI (none for C tables) must be
        /// readable before the table, so a Rust `OwnedObject` without an `RttiClass`
        /// needs `VtableHook::with_prefix(.., 0)` instead; every replacement must honor
        /// the declared signature, convention, and safety of its method, because
        /// callers may call safe methods without `unsafe`; no `&Vtbl` borrow of the
        /// active table may be alive while it is edited; `Patch` mode needs a writable
        /// table without concurrent callers; stacked hooks drop in reverse order.
        ///
        /// # Panics
        ///
        /// Panics if the vtable has no entries.
        #[must_use = "dropping the hook restores the original table"]
        #vis unsafe fn hook(
            &self,
            mode: #krate::hook::HookMode,
        ) -> #krate::hook::VtableHook<'_, Self> {
            unsafe { #krate::hook::VtableHook::new(self, mode) }
        }
    }
}

/// Tell if the documentation of a method already has a `# Safety` section.
fn has_safety_section(docs: &[syn::Attribute]) -> bool {
    docs.iter().any(|attr| match &attr.meta {
        syn::Meta::NameValue(pair) => matches!(
            &pair.value,
            syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(text), .. })
                if text.value().trim() == "# Safety"
        ),
        syn::Meta::Path(_) | syn::Meta::List(_) => false,
    })
}

/// Make one method of the interface type. The method calls through the vtable.
///
/// A method declared `fn` gives a safe caller; the `unsafe trait` declaration proves
/// that a live object makes it sound. A method declared `unsafe fn` gives an unsafe
/// caller with the documented preconditions.
fn caller_method(method: &Method, vis: &Visibility, variant: &AbiVariant) -> TokenStream {
    let docs = &method.docs;
    let attrs = &method.attrs;
    let unsafety = &method.unsafety;
    let name = &method.name;
    let names: Vec<&Ident> = method.params.iter().map(|param| &param.name).collect();
    let types = method.params.iter().map(|param| &param.ty);
    let expect = many_arguments_expect(method.params.len() + 1);
    let safety = (method.unsafety.is_some() && !has_safety_section(docs)).then(|| {
        quote! {
            ///
            /// # Safety
            ///
            /// The arguments must meet the preconditions that the declaration of this
            /// method documents.
        }
    });
    match effective_kind(method.kind, variant) {
        ReturnKind::Hidden => {
            let ret = return_type(method);
            let arguments = if variant.hidden_before_this {
                quote! { __cppvtable_result.as_mut_ptr(), self.as_raw() }
            } else {
                quote! { self.as_raw(), __cppvtable_result.as_mut_ptr() }
            };
            quote! {
                #(#docs)*
                #safety
                #(#attrs)*
                #expect
                #vis #unsafety fn #name(&self #(, #names: #types)*) -> #ret {
                    let mut __cppvtable_result = ::core::mem::MaybeUninit::<#ret>::uninit();
                    unsafe {
                        (self.vtable().#name)(#arguments #(, #names)*);
                        __cppvtable_result.assume_init()
                    }
                }
            }
        }
        ReturnKind::Scalar | ReturnKind::Unit | ReturnKind::Aggregate => {
            let output = &method.output;
            quote! {
                #(#docs)*
                #safety
                #(#attrs)*
                #expect
                #vis #unsafety fn #name(&self #(, #names: #types)*) #output {
                    unsafe { (self.vtable().#name)(self.as_raw() #(, #names)*) }
                }
            }
        }
    }
}

/// Select native RTTI representation without asserting that an object has RTTI.
fn cpp_abi_metadata(abi: Abi, abi_crate: &TokenStream) -> TokenStream {
    match abi {
        Abi::Cpp => quote! {
            const CPP_ABI: ::core::option::Option<#abi_crate::rtti::CppAbi> =
                ::core::option::Option::Some(#abi_crate::rtti::CppAbi::TARGET);
        },
        Abi::Msvc => quote! {
            const CPP_ABI: ::core::option::Option<#abi_crate::rtti::CppAbi> =
                ::core::option::Option::Some(#abi_crate::rtti::CppAbi::Msvc);
        },
        Abi::Itanium => quote! {
            const CPP_ABI: ::core::option::Option<#abi_crate::rtti::CppAbi> =
                ::core::option::Option::Some(#abi_crate::rtti::CppAbi::Itanium);
        },
        Abi::C | Abi::Com => TokenStream::new(),
    }
}

/// Make the implementation of the trait `ComInterface`.
fn com_interface_impl(
    args: &InterfaceArgs,
    model: &InterfaceModel,
    base: &Base,
    krate: &TokenStream,
) -> TokenStream {
    let name = &model.name;
    let allow = allow_deprecated(model);
    let guid = args.iid.as_ref().expect("COM arguments require an IID");
    let data1 = guid.data1;
    let data2 = guid.data2;
    let data3 = guid.data3;
    let data4 = guid.data4;
    let ancestors = match base.interface_type(krate) {
        None => quote! { &[] },
        Some(base_type) => quote! {
            {
                const LENGTH: usize = <#base_type as #krate::ComInterface>::ANCESTORS.len() + 1;
                const LIST: [#krate::GUID; LENGTH] = {
                    let mut list = [<#base_type as #krate::ComInterface>::IID; LENGTH];
                    let source = <#base_type as #krate::ComInterface>::ANCESTORS;
                    let mut index = 0;
                    while index < source.len() {
                        list[index + 1] = source[index];
                        index += 1;
                    }
                    list
                };
                &LIST
            }
        },
    };
    quote! {
        #allow
        unsafe impl #krate::ComInterface for #name {
            const IID: #krate::GUID = #krate::GUID::from_values(#data1, #data2, #data3, [#(#data4),*]);
            const ANCESTORS: &'static [#krate::GUID] = #ancestors;
        }
    }
}

/// Make the implementation of the trait `Interface`.
fn interface_trait_impl(
    args: &InterfaceArgs,
    model: &InterfaceModel,
    base: &Base,
    krate: &TokenStream,
    abi_crate: &TokenStream,
    vtbl_name: &Ident,
    is_plain: bool,
) -> TokenStream {
    let name = &model.name;
    let name_text = name.to_string();
    let allow = allow_deprecated(model);
    let cpp_abi = cpp_abi_metadata(args.abi, abi_crate);
    let layout = match args.layout {
        Layout::Pointer => quote! { #abi_crate::VtableLayout::Pointer },
        Layout::Inline => quote! { #abi_crate::VtableLayout::Inline },
    };
    let base_layout_check = base.interface_type(krate).map(|base| quote! {
        #allow
        const _: () = assert!(
            matches!(
                (<#base as #abi_crate::Interface>::LAYOUT, <#name as #abi_crate::Interface>::LAYOUT),
                (#abi_crate::VtableLayout::Pointer, #abi_crate::VtableLayout::Pointer)
                    | (#abi_crate::VtableLayout::Inline, #abi_crate::VtableLayout::Inline)
            ),
            "derived and base interfaces must use the same vtable layout"
        );
    });
    let inline_extent_check = (args.layout == Layout::Inline).then(|| {
        quote! {
            const _: () = assert!(
                ::core::mem::size_of::<#vtbl_name>() > 0,
                "an inline interface must have at least one function-pointer slot"
            );
        }
    });
    let extra = if args.abi.is_com() {
        com_interface_impl(args, model, base, krate)
    } else if is_plain {
        let matches_base = base.interface_type(krate).map(|ty| {
            quote! {
                || <#ty as #krate::CppInterface>::matches_type(id)
            }
        });
        let (storage_type, storage_value) = if args.layout == Layout::Inline {
            (quote! { #vtbl_name }, quote! { *vtable })
        } else {
            (
                quote! { #abi_crate::VtablePtr },
                quote! {
                    #abi_crate::VtablePtr::new(::core::ptr::from_ref(vtable).cast::<::core::ffi::c_void>())
                },
            )
        };
        quote! {
            #allow
            unsafe impl #krate::CppInterface for #name {
                type Storage = #storage_type;
                fn storage(vtable: &'static Self::Vtbl) -> Self::Storage {
                    #storage_value
                }
                fn matches_type(id: ::core::any::TypeId) -> bool {
                    id == ::core::any::TypeId::of::<Self>() #matches_base
                }
            }
        }
    } else {
        TokenStream::new()
    };
    quote! {
        #allow
        unsafe impl #abi_crate::Interface for #name {
            type Vtbl = #vtbl_name;
            const NAME: &'static str = #name_text;
            const LAYOUT: #abi_crate::VtableLayout = #layout;
            #cpp_abi
        }
        #base_layout_check
        #inline_extent_check
        #extra
    }
}

/// Make the `Deref` to the base interface.
fn deref_to_base(model: &InterfaceModel, base: &Base, krate: &TokenStream) -> TokenStream {
    let name = &model.name;
    let Some(base_type) = base.interface_type(krate) else {
        return TokenStream::new();
    };
    let allow = allow_deprecated(model);
    quote! {
        #allow
        impl ::core::ops::Deref for #name {
            type Target = #base_type;

            #[inline]
            fn deref(&self) -> &Self::Target {
                unsafe { &*::core::ptr::from_ref(self).cast::<Self::Target>() }
            }
        }
    }
}

/// Make the trait of the implementer.
fn impl_trait(
    model: &InterfaceModel,
    base: &Base,
    implementation: &TokenStream,
    impl_name: &Ident,
) -> TokenStream {
    let name = &model.name;
    let vis = &model.vis;
    let bound = base.impl_bound(implementation);
    let doc = format!(
        "The implementer side of [`{name}`].\n\n\
         Implement this trait for the type that `#[implement({name})]` marks. Each \
         method takes `&self` to permit reentrant foreign calls. Use interior mutability \
         for state that changes. Thread access must obey the interface and owning \
         object's contract; `&self` does not authorize arbitrary concurrent calls.\n\n\
         These methods can also be called directly on a standalone Rust value, and the \
         safe callers of [`{name}`] reach them through the vtable without `unsafe`. A \
         safe method must accept every argument permitted by its Rust signature and \
         cannot assume that `self` is embedded in an object allocation. Declare a method \
         `unsafe fn` and document its preconditions when it requires valid foreign \
         pointers or an embedded `self`. The declaration's method safety is preserved \
         exactly; pointer types do not imply unsafety automatically. Generated vtable \
         shims recover `self` from a live object allocation before invoking a method; \
         foreign callers must uphold the declared method preconditions. A panic that \
         reaches a shim aborts the process."
    );
    let methods = model.slots.iter().filter_map(|slot| {
        let method = slot.method.as_ref()?;
        let docs = &method.docs;
        let attrs = &method.attrs;
        let method_name = &method.name;
        let names = method.params.iter().map(|param| &param.name);
        let types = method.params.iter().map(|param| &param.ty);
        let output = &method.output;
        let unsafety = &method.unsafety;
        let expect = many_arguments_expect(method.params.len() + 1);
        Some(quote! {
            #(#docs)*
            #(#attrs)*
            #expect
            #unsafety fn #method_name(&self #(, #names: #types)*) #output;
        })
    });
    let expect = non_snake_case_expect(
        &model.method_names(),
        "The method names are the names of the foreign header.",
    );

    quote! {
        #[doc = #doc]
        #expect
        #vis trait #impl_name: #bound {
            #(#methods)*
        }
    }
}

/// Make the builder of a static vtable.
fn vtable_builder(
    model: &InterfaceModel,
    base: &Base,
    krate: &TokenStream,
    vtbl_name: &Ident,
    impl_name: &Ident,
    vis: &Visibility,
    trailing_slots: Option<&TokenStream>,
) -> TokenStream {
    let name = &model.name;
    let trailing_value = trailing_slots.map(|count| {
        quote! {
            __reserved_tail: [::core::option::Option::None; #count],
        }
    });
    let base_value = match base {
        Base::None => TokenStream::new(),
        Base::Unknown => quote! { base: #krate::IUnknownVtbl::new::<T, SLOT>(), },
        Base::Interface(path) => {
            let vtbl = derived_path(path, "Vtbl");
            quote! { base: #vtbl::new::<T, SLOT>(), }
        }
    };
    let values = model.slots.iter().map(|slot| {
        if let Some(method) = &slot.method {
            let field = &method.name;
            let shim = shim_name(name, slot.index);
            quote! { #field: #shim::<T, SLOT>, }
        } else {
            let field = format_ident!("reserved_{}", slot.index);
            quote! { #field: ::core::option::Option::None, }
        }
    });
    let doc = format!(
        "Make the vtable of [`{name}`] for the type `T`.\n\n\
         `SLOT` is the index of the vtable pointer of this interface chain inside the \
         object. The shims use the index for the `this` adjustment. `#[implement]` \
         calls this function in the initializer of a static."
    );

    quote! {
        impl #vtbl_name {
            #[doc = #doc]
            #[must_use]
            #vis const fn new<T: #impl_name, const SLOT: usize>() -> Self {
                Self {
                    #base_value
                    #(#values)*
                    #trailing_value
                }
            }
        }
    }
}

/// Give `#[expect(non_snake_case, ...)]` when at least one name breaks the rule.
///
/// `#[expect]` is an error when the lint does not fire, so the macro adds the attribute
/// only when it knows that a name breaks the rule.
fn non_snake_case_expect(names: &[String], reason: &str) -> TokenStream {
    if needs_non_snake_case(names.iter().map(String::as_str)) {
        quote! { #[expect(non_snake_case, reason = #reason)] }
    } else {
        TokenStream::new()
    }
}

/// Give `#[expect(clippy::too_many_arguments, ...)]` when the method has too many.
fn many_arguments_expect(count: usize) -> TokenStream {
    if count > ARGUMENT_LIMIT {
        quote! {
            #[expect(
                clippy::too_many_arguments,
                reason = "The signature of the foreign method is fixed."
            )]
        }
    } else {
        TokenStream::new()
    }
}
