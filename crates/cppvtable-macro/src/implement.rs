//! The code generator of `#[implement]`.
//!
//! For `#[implement(IFoo, IBar)] struct Foo { ... }` the macro makes:
//!
//! - One static vtable for each interface chain. The builder `IFooVtbl::new::<Foo, 0>()`
//!   fills it with the shims of the chain, from the methods of `IFoo` down to the three
//!   methods of `IUnknown`.
//! - One static array of the addresses of those vtables. `ComObject<Foo>` copies the
//!   array into each new object, and `ComPtr::as_impl` compares the vtable pointer of an
//!   object with the addresses in the array.
//! - `impl ComImplement for Foo`: the array type, the primary interface, and the table
//!   of `QueryInterface`.
//! - `impl Implements<IFoo> for Foo` with the index of the vtable pointer. The index is
//!   the `this` adjustment of the chain.
//!
//! The struct itself goes out without a change.

use proc_macro2::{Literal, TokenStream};
use quote::quote;
use syn::spanned::Spanned;
use syn::{Fields, ItemStruct};

use crate::parse::{ImplementArgs, derived_path, static_name};

/// Expand `#[implement(...)]`.
pub(crate) fn expand(args: TokenStream, item: &ItemStruct) -> Result<TokenStream, syn::Error> {
    let args = ImplementArgs::parse(args)?;
    if !item.generics.params.is_empty() {
        return Err(syn::Error::new(
            item.generics.span(),
            "an implementation type must not have a generic parameter and must not have \
             a lifetime. The static vtables need one fixed type.",
        ));
    }
    if matches!(item.fields, Fields::Unnamed(_)) {
        return Err(syn::Error::new(
            item.fields.span(),
            "use a structure with named fields or a unit structure",
        ));
    }
    Ok(generate(&args, item))
}

/// Make the output of the macro.
fn generate(args: &ImplementArgs, item: &ItemStruct) -> TokenStream {
    let krate = args.krate();
    let name = &item.ident;
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
            #krate::VtablePtr::new(
                ::core::ptr::from_ref(&#static_id).cast::<::core::ffi::c_void>()
            )
        }
    });

    let lookup = args.interfaces.iter().enumerate().map(|(slot, interface)| {
        let index = Literal::usize_unsuffixed(slot);
        quote! {
            if #krate::interface_matches::<#interface>(iid) {
                return ::core::option::Option::Some(#index);
            }
        }
    });

    let implements = args.interfaces.iter().enumerate().map(|(slot, interface)| {
        let index = Literal::usize_unsuffixed(slot);
        quote! {
            unsafe impl #krate::Implements<#interface> for #name {
                const SLOT: usize = #index;
            }
        }
    });

    let primary = &args.interfaces[0];
    let table_doc = format!(
        "The addresses of the static vtables of `{name_text}`. The order is the order of \
         the `#[implement]` list."
    );

    quote! {
        #item

        #(#vtable_statics)*

        #[doc = #table_doc]
        static #table_id: [#krate::VtablePtr; #count] = [#(#table_entries),*];

        unsafe impl #krate::ComImplement for #name {
            type Vtables = [#krate::VtablePtr; #count];
            type Primary = #primary;

            fn vtables() -> Self::Vtables {
                #table_id
            }

            fn vtable_slots() -> &'static [#krate::VtablePtr] {
                &#table_id
            }

            fn slot_for_iid(iid: &#krate::GUID) -> ::core::option::Option<usize> {
                #(#lookup)*
                ::core::option::Option::None
            }
        }

        #(#implements)*
    }
}

#[cfg(test)]
mod tests {
    use proc_macro2::TokenStream;
    use quote::quote;

    /// Expand a declaration that must be correct and give the tokens as text.
    fn expand_ok(args: TokenStream, item: TokenStream) -> String {
        let parsed: syn::ItemStruct = syn::parse2(item).unwrap();
        super::expand(args, &parsed).unwrap().to_string()
    }

    /// Expand a declaration that must fail and give the message.
    fn expand_err(args: TokenStream, item: TokenStream) -> String {
        let parsed: syn::ItemStruct = syn::parse2(item).unwrap();
        super::expand(args, &parsed).unwrap_err().to_string()
    }

    #[test]
    fn the_macro_makes_one_vtable_and_one_index_for_each_interface() {
        let text = expand_ok(
            quote! { IFoo, IBar },
            quote! {
                struct Thing { value: u32 }
            },
        );
        assert!(text.contains(
            "static CPPVTABLE_THING_VTBL_0 : IFooVtbl = IFooVtbl :: new :: < Thing , 0 > ()"
        ));
        assert!(text.contains(
            "static CPPVTABLE_THING_VTBL_1 : IBarVtbl = IBarVtbl :: new :: < Thing , 1 > ()"
        ));
        assert!(text.contains("static CPPVTABLE_THING_VTABLES : [:: cppvtable :: VtablePtr ; 2]"));
        assert!(text.contains("type Primary = IFoo"));
        assert!(text.contains(
            "impl :: cppvtable :: Implements < IFoo > for Thing { const SLOT : usize = 0 ; }"
        ));
        assert!(text.contains(
            "impl :: cppvtable :: Implements < IBar > for Thing { const SLOT : usize = 1 ; }"
        ));
        assert!(text.contains("interface_matches :: < IFoo > (iid)"));
        assert!(text.contains("interface_matches :: < IBar > (iid)"));
        // The structure goes out without a change.
        assert!(text.contains("struct Thing { value : u32 }"));
    }

    #[test]
    fn the_internal_flag_changes_the_paths() {
        let text = expand_ok(
            quote! { IFoo, internal },
            quote! {
                struct Thing;
            },
        );
        assert!(text.contains("impl crate :: ComImplement for Thing"));
        assert!(!text.contains(":: cppvtable ::"));
    }

    #[test]
    fn a_bad_declaration_gives_a_clear_error() {
        assert!(expand_err(quote! {}, quote! { struct Thing; }).contains("at least one interface"));
        assert!(
            expand_err(quote! { IFoo }, quote! { struct Thing<T> { value: T } })
                .contains("generic parameter")
        );
        assert!(
            expand_err(quote! { IFoo }, quote! { struct Thing(u32); }).contains("named fields")
        );
    }
}
