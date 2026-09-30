//! The code generator of `#[implement]`.
//!
//! For `#[implement(IFoo, IBar)] struct Foo { ... }` the macro makes:
//!
//! - One static vtable for each interface chain. `IFooVtbl::new::<Foo, 0>()` fills
//!   it with the shims of that interface and its bases.
//! - One static array of vtable addresses, copied into `Object<Foo>` or `ComObject<Foo>`.
//! - `impl Implement for Foo` for ordinary objects, or `impl ComImplement for Foo`
//!   for COM objects. Ordinary interface lookup uses Rust type identity; COM uses IIDs
//!   and includes `IUnknown` behavior.
//! - `impl Implements<IFoo> for Foo` with the index of the vtable pointer. The index is
//!   the `this` adjustment of the chain.
//! - With `refcount = single` or `refcount = dual` (COM only), `impl RefCounted for Foo`
//!   with that policy and the default hooks, plus a compile-time check that `Foo` is
//!   `Send + Sync` when an implemented interface or one of its bases is an
//!   `AgileInterface`.
//!
//! The struct itself goes out without a change. Named, tuple, and unit structures work.

use proc_macro2::{Literal, Span, TokenStream};
use quote::{format_ident, quote};
use syn::ItemStruct;
use syn::spanned::Spanned;

use crate::parse::{ImplementArgs, RefCount, Runtime, derived_name, derived_path, static_name};

/// Expand `#[implement(...)]` for the standalone COM runtime package.
pub(crate) fn expand(args: TokenStream, item: &ItemStruct) -> Result<TokenStream, syn::Error> {
    expand_with_runtime(args, item, Runtime::Com)
}

/// Expand `#[implement(...)]` for ordinary C/C++ objects.
pub(crate) fn expand_native(
    args: TokenStream,
    item: &ItemStruct,
) -> Result<TokenStream, syn::Error> {
    expand_with_runtime(args, item, Runtime::Native)
}

fn expand_with_runtime(
    args: TokenStream,
    item: &ItemStruct,
    runtime: Runtime,
) -> Result<TokenStream, syn::Error> {
    let args = ImplementArgs::parse(args)?;
    if !item.generics.params.is_empty() {
        return Err(syn::Error::new(
            item.generics.span(),
            "an implementation type must not have a generic parameter and must not have \
             a lifetime. The static vtables need one fixed type.",
        ));
    }
    if args.refcount.is_some() && matches!(runtime, Runtime::Native) {
        return Err(syn::Error::new(
            Span::call_site(),
            "refcount is a COM option; ordinary objects are owned by `OwnedObject`",
        ));
    }
    Ok(generate(&args, item, runtime))
}

/// Make the output of the macro.
fn generate(args: &ImplementArgs, item: &ItemStruct, runtime: Runtime) -> TokenStream {
    let krate = args.krate(runtime);
    let abi_krate = if args.internal {
        krate.clone()
    } else {
        ImplementArgs::abi_krate(runtime)
    };
    let name = &item.ident;
    let is_plain = matches!(runtime, Runtime::Native);
    let implementation = if is_plain {
        quote! { #krate::Implement }
    } else {
        quote! { #krate::ComImplement }
    };
    let name_text = name.to_string();
    let count = Literal::usize_unsuffixed(args.interfaces.len());
    let table_id = static_name(name, "VTABLES");

    let vtable_statics = args.interfaces.iter().enumerate().map(|(slot, interface)| {
        let static_id = static_name(name, &format!("VTBL_{slot}"));
        let vtbl = derived_path(interface, "Vtbl");
        let index = Literal::usize_unsuffixed(slot);
        let doc = format!("The static vtable of `{name_text}` for the interface chain {slot}.");
        quote! {
            #[doc = #doc]
            static #static_id: #vtbl = #vtbl::new::<#name, #index>();
        }
    });

    let table_entries = (0..args.interfaces.len()).map(|slot| {
        let static_id = static_name(name, &format!("VTBL_{slot}"));
        quote! {
            #abi_krate::VtablePtr::new(
                ::core::ptr::from_ref(&#static_id).cast::<::core::ffi::c_void>()
            )
        }
    });

    let lookup_method = lookup_method(args, &krate, is_plain);

    let implements = args.interfaces.iter().enumerate().map(|(slot, interface)| {
        let index = Literal::usize_unsuffixed(slot);
        quote! {
            unsafe impl #krate::Implements<#interface> for #name {
                const SLOT: usize = #index;
            }
        }
    });

    let primary = &args.interfaces[0];
    let (storage_declaration, storage_type, storage_value, slot_offsets) =
        storage_plan(args, item, &krate, &abi_krate, is_plain);
    let refcount = args
        .refcount
        .map(|policy| refcounted(args, name, &krate, policy));
    let table_doc = format!(
        "The addresses of the static vtables of `{name_text}`. The order is the order of \
         the `#[implement]` list."
    );

    quote! {
        #item

        #storage_declaration
        #(#vtable_statics)*

        #[doc = #table_doc]
        static #table_id: [#abi_krate::VtablePtr; #count] = [#(#table_entries),*];

        unsafe impl #implementation for #name {
            type Vtables = #storage_type;
            type Primary = #primary;
            #slot_offsets

            fn vtables() -> Self::Vtables {
                #storage_value
            }

            fn vtable_slots() -> &'static [#abi_krate::VtablePtr] {
                &#table_id
            }

            #lookup_method
        }

        #(#implements)*
        #refcount
    }
}

/// Make the method that maps an interface identity to the index of its vtable pointer.
fn lookup_method(args: &ImplementArgs, krate: &TokenStream, is_plain: bool) -> TokenStream {
    let lookup = args.interfaces.iter().enumerate().map(|(slot, interface)| {
        let index = Literal::usize_unsuffixed(slot);
        if is_plain {
            quote! {
                if <#interface as #krate::CppInterface>::matches_type(id) {
                    return ::core::option::Option::Some(#index);
                }
            }
        } else {
            quote! {
                if #krate::interface_matches::<#interface>(iid) {
                    return ::core::option::Option::Some(#index);
                }
            }
        }
    });
    if is_plain {
        quote! {
            fn slot_for_type(id: ::core::any::TypeId) -> ::core::option::Option<usize> {
                #(#lookup)*
                ::core::option::Option::None
            }
        }
    } else {
        quote! {
            fn slot_for_iid(iid: &#krate::GUID) -> ::core::option::Option<usize> {
                #(#lookup)*
                ::core::option::Option::None
            }
        }
    }
}

/// Implement `RefCounted` with a standard policy and the default hooks.
///
/// The default hooks return no pointers and do not touch the count, so the remaining
/// obligation of `RefCounted` is `Send + Sync` for an object behind an
/// `AgileInterface`. Each implemented interface is probed by method resolution: the
/// probe method of the first agile interface along the `Deref` chain from that
/// interface to `IUnknown` requires `Send + Sync`, and only `IUnknown` supplies the
/// unconstrained fallback. The probe is type-checked and never runs.
fn refcounted(
    args: &ImplementArgs,
    name: &syn::Ident,
    krate: &TokenStream,
    policy: RefCount,
) -> TokenStream {
    let policy = match policy {
        RefCount::Single => quote! { #krate::SingleRefCount },
        RefCount::Dual => quote! { #krate::DualRefCount },
    };
    let interfaces = &args.interfaces;
    quote! {
        unsafe impl #krate::RefCounted for #name {
            type Policy = #policy;
        }

        const _: () = {
            trait AgileInterfaceRequiresSendSync {
                fn agile_interface_requires_send_sync<T>(
                    &self,
                    _object: ::core::marker::PhantomData<T>,
                ) where
                    T: ::core::marker::Send + ::core::marker::Sync,
                {
                }
            }
            impl<I: #krate::AgileInterface> AgileInterfaceRequiresSendSync for I {}

            trait NoAgileInterface {
                fn agile_interface_requires_send_sync<T>(
                    &self,
                    _object: ::core::marker::PhantomData<T>,
                ) {
                }
            }
            impl NoAgileInterface for #krate::IUnknown {}

            #(
                let _ = |interface: &#interfaces| {
                    interface.agile_interface_requires_send_sync(
                        ::core::marker::PhantomData::<#name>,
                    );
                };
            )*
        };
    }
}

/// Construct either native per-interface headers or the COM vtable-pointer array.
fn storage_plan(
    args: &ImplementArgs,
    item: &ItemStruct,
    krate: &TokenStream,
    abi_krate: &TokenStream,
    is_plain: bool,
) -> (TokenStream, TokenStream, TokenStream, TokenStream) {
    let name = &item.ident;
    let count = Literal::usize_unsuffixed(args.interfaces.len());
    let table_id = static_name(name, "VTABLES");
    let storage_name = derived_name(name, "Vtables");
    if is_plain {
        let fields = args.interfaces.iter().enumerate().map(|(slot, interface)| {
            let field = format_ident!("slot_{slot}");
            quote! { #field: <#interface as #krate::CppInterface>::Storage, }
        });
        let values = args.interfaces.iter().enumerate().map(|(slot, interface)| {
            let field = format_ident!("slot_{slot}");
            let static_id = static_name(name, &format!("VTBL_{slot}"));
            quote! { #field: <#interface as #krate::CppInterface>::storage(&#static_id), }
        });
        let offsets = (0..args.interfaces.len()).map(|slot| {
            let field = format_ident!("slot_{slot}");
            quote! { ::core::mem::offset_of!(#storage_name, #field) }
        });
        let descriptors = args.interfaces.iter().map(|interface| {
            quote! {
                #krate::InterfaceDescriptor {
                    layout: <#interface as #abi_krate::Interface>::LAYOUT,
                    cpp_abi: <#interface as #abi_krate::Interface>::CPP_ABI,
                    table_size: ::core::mem::size_of::<<#interface as #abi_krate::Interface>::Vtbl>(),
                    table_align: ::core::mem::align_of::<<#interface as #abi_krate::Interface>::Vtbl>(),
                }
            }
        });
        let vis = &item.vis;
        (
            quote! {
                /// Interface headers stored at the start of the generated object allocation.
                #[repr(C)]
                #[derive(Clone, Copy)]
                #vis struct #storage_name { #(#fields)* }
            },
            quote! { #storage_name },
            quote! { #storage_name { #(#values)* } },
            quote! {
                const SLOT_OFFSETS: &'static [usize] = &[#(#offsets),*];
                const INTERFACES: &'static [#krate::InterfaceDescriptor] = &[#(#descriptors),*];
            },
        )
    } else {
        (
            TokenStream::new(),
            quote! { [#abi_krate::VtablePtr; #count] },
            quote! { #table_id },
            TokenStream::new(),
        )
    }
}

#[cfg(test)]
mod tests {
    use proc_macro2::TokenStream;
    use quote::quote;

    fn expand_err(args: TokenStream, item: TokenStream) -> String {
        let parsed: syn::ItemStruct = syn::parse2(item).unwrap();
        super::expand(args, &parsed).unwrap_err().to_string()
    }

    #[test]
    fn a_bad_declaration_gives_a_clear_error() {
        assert!(expand_err(quote! {}, quote! { struct Thing; }).contains("at least one interface"));
        assert!(
            expand_err(quote! { IFoo }, quote! { struct Thing<T> { value: T } })
                .contains("generic parameter")
        );
        assert!(
            expand_err(
                quote! { IFoo, refcount = forward },
                quote! { struct Thing; }
            )
            .contains("`single` or `dual`")
        );
        assert!(
            expand_err(
                quote! { IFoo, refcount = single, refcount = dual },
                quote! { struct Thing; }
            )
            .contains("only once")
        );
        let native = super::expand_native(
            quote! { IFoo, refcount = single },
            &syn::parse_quote! { struct Thing; },
        );
        assert!(native.unwrap_err().to_string().contains("COM option"));
    }

    #[test]
    fn tuple_structures_are_implementable() {
        for expand in [super::expand, super::expand_native] {
            let output = expand(quote! { IFoo }, &syn::parse_quote! { struct Thing(u32); })
                .unwrap()
                .to_string();
            assert!(output.contains("struct Thing (u32) ;"));
            assert!(output.contains("Implements < IFoo > for Thing"));
        }
    }

    #[test]
    fn refcount_writes_the_policy_and_the_agile_probe() {
        for (argument, policy) in [
            (quote! { single }, "SingleRefCount"),
            (quote! { dual }, "DualRefCount"),
        ] {
            let output = super::expand(
                quote! { IFoo, IBar, refcount = #argument },
                &syn::parse_quote! { struct Thing; },
            )
            .unwrap()
            .to_string();
            assert!(output.contains(&format!(
                "unsafe impl :: cppvtable_com :: RefCounted for Thing {{ type Policy = :: cppvtable_com :: {policy} ; }}"
            )));
            assert!(output.contains("impl < I : :: cppvtable_com :: AgileInterface >"));
            assert!(output.contains("impl NoAgileInterface for :: cppvtable_com :: IUnknown"));
            assert!(output.contains("| interface : & IFoo |"));
            assert!(output.contains("| interface : & IBar |"));
        }
        let manual = super::expand(quote! { IFoo }, &syn::parse_quote! { struct Thing; })
            .unwrap()
            .to_string();
        assert!(!manual.contains("RefCounted"));
    }
}
