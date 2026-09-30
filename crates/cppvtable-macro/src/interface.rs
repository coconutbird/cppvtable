//! The code generator of `#[interface]`.
//!
//! For `IFoo` the macro makes:
//!
//! - `IFoo`: the interface type. It is a transparent wrapper of one interface pointer.
//!   It has one method for each declared method. A method calls through the vtable.
//! - `IFooVtbl`: the `#[repr(C)]` vtable structure. The first field is the vtable of the
//!   base interface.
//! - `impl Interface for IFoo`: the vtable type, the IID, and the IIDs of the chain.
//! - `impl Deref for IFoo`: the base interface. A `ComPtr<IFoo>` therefore also gives
//!   the methods of each base interface.
//! - `IFooImpl`: the trait of the implementer. Its supertrait is the `Impl` trait of the
//!   base interface.
//! - `IFooVtbl::new::<T, SLOT>()`: the builder of a static vtable for the type `T`.
//! - One shim for each method. A shim moves `this` back to the start of the allocation
//!   and calls the method of `IFooImpl`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, ItemTrait, Path, Visibility};

use crate::abi::AbiVariant;
use crate::parse::{InterfaceArgs, InterfaceModel, Method, derived_name, derived_path, shim_name};
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

/// Expand `#[interface(...)]`.
pub(crate) fn expand(args: TokenStream, item: &ItemTrait) -> Result<TokenStream, syn::Error> {
    let args = InterfaceArgs::parse(args)?;
    let model = InterfaceModel::parse(item)?;
    Ok(generate(&args, &model))
}

/// Make the whole output of the macro.
fn generate(args: &InterfaceArgs, model: &InterfaceModel) -> TokenStream {
    let krate = args.krate();
    let name = &model.name;
    let vis = &model.vis;
    let vtbl_name = derived_name(name, "Vtbl");
    let impl_name = derived_name(name, "Impl");
    let base = base_of(args);

    let mut output = TokenStream::new();
    for variant in args.abi.variants() {
        output.extend(vtable_struct(model, &variant, &base, &krate, &vtbl_name));
        if !args.root {
            output.extend(shims(model, &variant, &krate, &impl_name));
        }
    }
    output.extend(interface_type(model, &krate, &vtbl_name));
    output.extend(interface_trait_impl(args, model, &base, &krate, &vtbl_name));
    output.extend(deref_to_base(model, &base, &krate));
    if !args.root {
        output.extend(impl_trait(model, &base, &krate, &impl_name));
        output.extend(vtable_builder(
            model, &base, &krate, &vtbl_name, &impl_name, vis,
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
    fn impl_bound(&self, krate: &TokenStream) -> TokenStream {
        match self {
            Self::None | Self::Unknown => quote! { #krate::ComImplement },
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
) -> TokenStream {
    let vis = &model.vis;
    let name = &model.name;
    let cfg = &variant.cfg;
    let convention = variant.convention;
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
            let ty = pointer_type(method, convention);
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
        #vis struct #vtbl_name {
            #base_field
            #(#fields)*
        }
    }
}

/// Give the type of the function pointer of a method.
fn pointer_type(method: &Method, convention: &str) -> TokenStream {
    let names = method.params.iter().map(|param| &param.name);
    let types = method.params.iter().map(|param| &param.ty);
    match method.kind {
        ReturnKind::Hidden => {
            let ret = return_type(method);
            quote! {
                unsafe extern #convention fn(
                    this: *mut ::core::ffi::c_void,
                    result: *mut #ret,
                    #(#names: #types),*
                ) -> *mut #ret
            }
        }
        ReturnKind::Scalar | ReturnKind::Unit => {
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
    krate: &TokenStream,
    impl_name: &Ident,
) -> TokenStream {
    let cfg = &variant.cfg;
    let convention = variant.convention;
    let name = &model.name;
    let items = model.slots.iter().filter_map(|slot| {
        let method = slot.method.as_ref()?;
        let shim = shim_name(name, slot.index);
        let method_name = &method.name;
        let names: Vec<&Ident> = method.params.iter().map(|param| &param.name).collect();
        let types = method.params.iter().map(|param| &param.ty);
        let doc = format!("The shim of [`{name}::{method_name}`].");
        Some(match method.kind {
            ReturnKind::Hidden => {
                let ret = return_type(method);
                let expect = many_arguments_expect(method.params.len() + 2);
                quote! {
                    #[doc = #doc]
                    #cfg
                    #expect
                    unsafe extern #convention fn #shim<T: #impl_name, const SLOT: usize>(
                        this: *mut ::core::ffi::c_void,
                        result: *mut #ret,
                        #(#names: #types),*
                    ) -> *mut #ret {
                        unsafe {
                            let object = #krate::ComObject::<T>::impl_from_slot(this, SLOT);
                            let value = <T as #impl_name>::#method_name(object #(, #names)*);
                            ::core::ptr::write(result, value);
                            result
                        }
                    }
                }
            }
            ReturnKind::Scalar | ReturnKind::Unit => {
                let output = &method.output;
                let expect = many_arguments_expect(method.params.len() + 1);
                quote! {
                    #[doc = #doc]
                    #cfg
                    #expect
                    unsafe extern #convention fn #shim<T: #impl_name, const SLOT: usize>(
                        this: *mut ::core::ffi::c_void,
                        #(#names: #types),*
                    ) #output {
                        unsafe {
                            let object = #krate::ComObject::<T>::impl_from_slot(this, SLOT);
                            <T as #impl_name>::#method_name(object #(, #names)*)
                        }
                    }
                }
            }
        })
    });
    quote! { #(#items)* }
}

/// Make the interface type and its methods.
fn interface_type(model: &InterfaceModel, krate: &TokenStream, vtbl_name: &Ident) -> TokenStream {
    let name = &model.name;
    let vis = &model.vis;
    let docs = &model.docs;
    let type_doc = format!(
        "The interface `{name}`.\n\n\
         The type is a transparent wrapper of one interface pointer. A `ComPtr<{name}>` \
         owns a public reference and derefs to this type, so the methods below and the \
         methods of each base interface are available."
    );

    let methods = model.slots.iter().filter_map(|slot| {
        let method = slot.method.as_ref()?;
        Some(caller_method(method, krate, vis))
    });

    let expect = non_snake_case_expect(
        &model.method_names(),
        "The method names are the names of the foreign header.",
    );

    quote! {
        #(#docs)*
        #[doc = ""]
        #[doc = #type_doc]
        #[repr(transparent)]
        #vis struct #name(::core::ptr::NonNull<::core::ffi::c_void>);

        #expect
        impl #name {
            /// Give the raw interface pointer.
            #[inline]
            #[must_use]
            #vis const fn as_raw(&self) -> *mut ::core::ffi::c_void {
                self.0.as_ptr()
            }

            /// Borrow a raw interface pointer as an interface reference.
            ///
            /// Use it to call the methods of an object without a change of a count.
            ///
            /// # Safety
            ///
            /// The place must hold a valid interface pointer of this interface, and the
            /// object must be alive during the life of the reference.
            #[inline]
            #[must_use]
            #vis unsafe fn from_raw_ref(place: &*mut ::core::ffi::c_void) -> &Self {
                unsafe { &*::core::ptr::from_ref(place).cast::<Self>() }
            }

            /// Give the vtable of the object.
            #[inline]
            #[must_use]
            #vis fn vtable(&self) -> *const #vtbl_name {
                #krate::vtable_of::<Self>(self)
            }

            #(#methods)*
        }
    }
}

/// Make one method of the interface type. The method calls through the vtable.
fn caller_method(method: &Method, krate: &TokenStream, vis: &Visibility) -> TokenStream {
    let docs = &method.docs;
    let name = &method.name;
    let names: Vec<&Ident> = method.params.iter().map(|param| &param.name).collect();
    let types = method.params.iter().map(|param| &param.ty);
    let expect = many_arguments_expect(method.params.len() + 1);
    match method.kind {
        ReturnKind::Hidden => {
            let ret = return_type(method);
            quote! {
                #(#docs)*
                ///
                /// # Safety
                ///
                /// The call goes through the vtable of a foreign object. The object must
                /// be alive, and each argument must obey the rules of the interface. The
                /// method uses the hidden return pointer of the MSVC ABI.
                #expect
                #vis unsafe fn #name(&self #(, #names: #types)*) -> #ret {
                    unsafe {
                        let mut result = ::core::mem::MaybeUninit::<#ret>::uninit();
                        ((*#krate::vtable_of::<Self>(self)).#name)(
                            #krate::raw_of::<Self>(self),
                            result.as_mut_ptr()
                            #(, #names)*
                        );
                        result.assume_init()
                    }
                }
            }
        }
        ReturnKind::Scalar | ReturnKind::Unit => {
            let output = &method.output;
            quote! {
                #(#docs)*
                ///
                /// # Safety
                ///
                /// The call goes through the vtable of a foreign object. The object must
                /// be alive, and each argument must obey the rules of the interface.
                #expect
                #vis unsafe fn #name(&self #(, #names: #types)*) #output {
                    unsafe {
                        ((*#krate::vtable_of::<Self>(self)).#name)(
                            #krate::raw_of::<Self>(self)
                            #(, #names)*
                        )
                    }
                }
            }
        }
    }
}

/// Make the implementation of the trait `Interface`.
fn interface_trait_impl(
    args: &InterfaceArgs,
    model: &InterfaceModel,
    base: &Base,
    krate: &TokenStream,
    vtbl_name: &Ident,
) -> TokenStream {
    let name = &model.name;
    let name_text = name.to_string();
    let is_com = args.abi.is_com();
    let iid = if let Some(guid) = &args.iid {
        let data1 = guid.data1;
        let data2 = guid.data2;
        let data3 = guid.data3;
        let data4 = guid.data4;
        quote! { #krate::GUID::from_values(#data1, #data2, #data3, [#(#data4),*]) }
    } else {
        quote! { #krate::GUID::from_values(0, 0, 0, [0, 0, 0, 0, 0, 0, 0, 0]) }
    };
    let ancestors = match base.interface_type(krate) {
        None => quote! { &[] },
        Some(base_type) => quote! {
            {
                const LENGTH: usize =
                    <#base_type as #krate::Interface>::ANCESTORS.len() + 1;
                const LIST: [#krate::GUID; LENGTH] = {
                    let mut list = [<#base_type as #krate::Interface>::IID; LENGTH];
                    let source = <#base_type as #krate::Interface>::ANCESTORS;
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
        unsafe impl #krate::Interface for #name {
            type Vtbl = #vtbl_name;
            const IID: #krate::GUID = #iid;
            const ANCESTORS: &'static [#krate::GUID] = #ancestors;
            const IS_COM: bool = #is_com;
            const NAME: &'static str = #name_text;
        }
    }
}

/// Make the `Deref` to the base interface.
fn deref_to_base(model: &InterfaceModel, base: &Base, krate: &TokenStream) -> TokenStream {
    let name = &model.name;
    let Some(base_type) = base.interface_type(krate) else {
        return TokenStream::new();
    };
    quote! {
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
    krate: &TokenStream,
    impl_name: &Ident,
) -> TokenStream {
    let name = &model.name;
    let vis = &model.vis;
    let bound = base.impl_bound(krate);
    let doc = format!(
        "The implementer side of [`{name}`].\n\n\
         Implement this trait for the type that `#[implement({name})]` marks. Each \
         method takes `&self`, because a foreign caller can call the object again \
         during a call and from another thread. Use interior mutability for state that \
         changes."
    );
    let methods = model.slots.iter().filter_map(|slot| {
        let method = slot.method.as_ref()?;
        let docs = &method.docs;
        let method_name = &method.name;
        let names = method.params.iter().map(|param| &param.name);
        let types = method.params.iter().map(|param| &param.ty);
        let output = &method.output;
        let expect = many_arguments_expect(method.params.len() + 1);
        Some(quote! {
            #(#docs)*
            #expect
            fn #method_name(&self #(, #names: #types)*) #output;
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
) -> TokenStream {
    let name = &model.name;
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
