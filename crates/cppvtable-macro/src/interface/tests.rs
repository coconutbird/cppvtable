//! Validation tests for interface declarations.

use proc_macro2::TokenStream;
use quote::quote;

fn expand_err(args: TokenStream, item: TokenStream) -> String {
    let parsed: syn::ItemTrait = syn::parse2(item).unwrap();
    super::expand(args, &parsed).unwrap_err().to_string()
}

fn com_trait() -> TokenStream {
    quote! {
        pub unsafe trait IFoo {
            fn GetValue(&self, value: *mut u32) -> HRESULT;
            fn Release2(&self) -> u32;
        }
    }
}

#[test]
fn a_bad_argument_gives_a_clear_error() {
    let trait_item = com_trait();
    assert!(
        expand_err(
            quote! { abi = stdcall, iid = "00112233-4455-6677-8899-aabbccddeeff" },
            trait_item.clone()
        )
        .contains("`stdcall` is unknown")
    );
    assert!(expand_err(quote! {}, trait_item.clone()).contains("give the binary interface"));
    assert!(expand_err(quote! { abi = com }, trait_item.clone()).contains("needs `iid"));
    assert!(expand_err(quote! { abi = com, iid = "bad" }, trait_item.clone()).contains("iid:"));
    assert!(
        expand_err(
            quote! { abi = com, iid = "00112233-4455-6677-8899-aabbccddeeff", root, extends(IBase) },
            trait_item.clone()
        )
        .contains("do not go together")
    );
    assert!(
        expand_err(quote! { abi = c, unknown_option }, trait_item).contains("unknown argument")
    );
}

#[test]
fn a_bad_declaration_gives_a_clear_error() {
    let cases: [(TokenStream, &str); 8] = [
        (
            quote! {
                pub unsafe trait ITable<T> {
                    fn get(&self) -> u32;
                }
            },
            "generic parameters",
        ),
        (
            quote! {
                pub unsafe trait ITable: Clone {
                    fn get(&self) -> u32;
                }
            },
            "not a supertrait",
        ),
        (
            quote! {
                pub unsafe trait ITable {
                    type Item;
                }
            },
            "methods only",
        ),
        (
            quote! {
                pub unsafe trait ITable {
                    fn get(&self) -> u32 { 0 }
                }
            },
            "no body",
        ),
        (
            quote! {
                pub unsafe trait ITable {
                    fn get(&mut self) -> u32;
                }
            },
            "use `&self`",
        ),
        (
            quote! {
                pub unsafe trait ITable {
                    async fn get(&self) -> u32;
                }
            },
            "async",
        ),
        (
            quote! {
                pub unsafe trait ITable {
                    fn get(&self) -> BoundingBox;
                }
            },
            "hidden_return",
        ),
        (
            quote! {
                pub unsafe trait ITable {
                    #[inline]
                    fn get(&self) -> u32;
                }
            },
            "documentation only",
        ),
    ];
    for (item, expected) in cases {
        let message = super::expand_native(quote! { abi = c }, &syn::parse2(item).unwrap())
            .unwrap_err()
            .to_string();
        assert!(
            message.contains(expected),
            "the message `{message}` must hold `{expected}`"
        );
    }
}

#[test]
fn a_slot_that_is_already_in_use_gives_an_error() {
    let message = super::expand_native(
        quote! { abi = c },
        &syn::parse2(quote! {
            pub unsafe trait ITable {
                fn first(&self);
                fn second(&self);
                #[slot(1)]
                fn third(&self);
            }
        })
        .unwrap(),
    )
    .unwrap_err()
    .to_string();
    assert!(message.contains("slot(1) is already in use"));
    assert!(message.contains("next free slot of this interface is 2"));
}

#[test]
fn abi_entry_point_rejects_com_interfaces() {
    let item = syn::parse2::<syn::ItemTrait>(com_trait()).unwrap();
    let error = super::expand_abi(
        quote! { abi = com, iid = "00112233-4455-6677-8899-aabbccddeeff" },
        &item,
    )
    .unwrap_err();
    assert!(error.to_string().contains("ABI crate declares only"));
}

#[test]
fn runtime_namespaces_do_not_mix_com_and_ordinary_interfaces() {
    let item: syn::ItemTrait = syn::parse_quote!(
        unsafe trait IValue {
            fn value(&self) -> u32;
        }
    );
    assert!(
        super::expand_native(
            quote! { abi = com, iid = "00000000-0000-0000-C000-000000000046" },
            &item
        )
        .unwrap_err()
        .to_string()
        .contains("cppvtable_com::interface")
    );
    assert!(
        super::expand(quote! { abi = cpp }, &item)
            .unwrap_err()
            .to_string()
            .contains("cppvtable::interface")
    );
    assert!(
        super::expand_native(
            quote! { abi = c, iid = "00000000-0000-0000-C000-000000000046" },
            &item
        )
        .unwrap_err()
        .to_string()
        .contains("COM metadata")
    );
}

#[test]
fn explicit_abi_selection_generates_a_target_guard() {
    let item: syn::ItemTrait = syn::parse_quote!(
        unsafe trait IValue {
            fn value(&self) -> u32;
        }
    );
    for abi in [quote! { msvc }, quote! { itanium }] {
        let expanded = super::expand_native(quote! { abi = #abi }, &item)
            .unwrap()
            .to_string();
        assert!(expanded.contains("compile_error"));
        assert!(expanded.contains("target default"));
    }
}

#[test]
fn aggregate_return_attributes_cannot_conflict() {
    let item: syn::ItemTrait = syn::parse_quote!(
        unsafe trait IValue {
            #[abi(aggregate, hidden_return)]
            fn value(&self) -> Value;
        }
    );
    assert!(
        super::expand_native(quote! { abi = cpp }, &item)
            .unwrap_err()
            .to_string()
            .contains("aggregate cannot be combined")
    );
}
