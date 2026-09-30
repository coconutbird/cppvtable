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

fn assert_declared_method_safety(output: TokenStream) {
    let file: syn::File = syn::parse2(output).unwrap();
    let implementation = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Trait(item) if item.ident == "IContractImpl" => Some(item),
            _ => None,
        })
        .unwrap();
    let expected = [
        ("scalar", false),
        ("passthrough", false),
        ("read", true),
        ("write", true),
        ("protocol_state", true),
        ("aggregate", true),
        ("indirect", true),
    ];
    for (name, is_unsafe) in expected {
        let method = implementation
            .items
            .iter()
            .find_map(|item| match item {
                syn::TraitItem::Fn(item) if item.sig.ident == name => Some(item),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            matches!(method.sig.safety, syn::Safety::Unsafe(_)),
            is_unsafe,
            "method {name}"
        );
        let mut callers = 0;
        for item in &file.items {
            let syn::Item::Impl(item) = item else {
                continue;
            };
            let syn::Type::Path(ty) = item.self_ty.as_ref() else {
                continue;
            };
            if !ty.path.is_ident("IContract") {
                continue;
            }
            for item in &item.items {
                if let syn::ImplItem::Fn(item) = item {
                    if item.sig.ident == name {
                        callers += 1;
                        assert!(
                            matches!(item.sig.safety, syn::Safety::Unsafe(_)),
                            "caller {name} must remain unsafe"
                        );
                    }
                }
            }
        }
        assert!(callers > 0, "missing caller {name}");
    }
}

fn mixed_safety_contract() -> syn::ItemTrait {
    syn::parse_quote! {
        unsafe trait IContract {
            fn scalar(&self) -> u32;
            fn passthrough(&self, pointer: *mut u32) -> *mut u32;
            unsafe fn read(&self, input: *const u32) -> u32;
            unsafe fn write(&self, output: *mut u32);
            unsafe fn protocol_state(&self) -> u32;
            #[abi(aggregate)]
            unsafe fn aggregate(&self) -> Aggregate;
            #[abi(hidden_return)]
            unsafe fn indirect(&self) -> Aggregate;
        }
    }
}

#[test]
fn native_implementation_traits_preserve_declared_method_safety() {
    let item = mixed_safety_contract();
    for args in [
        quote! { abi = c },
        quote! { abi = cpp },
        quote! { abi = msvc },
        quote! { abi = itanium },
    ] {
        assert_declared_method_safety(super::expand_native(args, &item).unwrap());
    }
}

#[test]
fn com_implementation_traits_preserve_declared_method_safety() {
    assert_declared_method_safety(
        super::expand(
            quote! { abi = com, iid = "00000000-0000-0000-C000-000000000046" },
            &mixed_safety_contract(),
        )
        .unwrap(),
    );
}

#[test]
fn method_convention_override_matches_the_vtable_field_and_shim() {
    for convention in [
        "C", "system", "cdecl", "stdcall", "fastcall", "thiscall", "win64", "sysv64", "aapcs",
    ] {
        let declaration: syn::ItemTrait = syn::parse2(quote! {
            unsafe trait IMixed {
                fn default_method(&self) -> u32;
                #[abi(convention = #convention)]
                fn explicit_method(&self) -> u32;
                #[abi(aggregate, convention = #convention)]
                unsafe fn explicit_aggregate(&self) -> Aggregate;
            }
        })
        .unwrap();
        let file: syn::File =
            syn::parse2(super::expand_native(quote! { abi = c }, &declaration).unwrap()).unwrap();
        for (method, shim) in [
            ("explicit_method", "__cppvtable_imixed_slot_1"),
            ("explicit_aggregate", "__cppvtable_imixed_slot_2"),
        ] {
            let table = file
                .items
                .iter()
                .find_map(|item| match item {
                    syn::Item::Struct(item) if item.ident == "IMixedVtbl" => Some(item),
                    _ => None,
                })
                .unwrap();
            let field = table
                .fields
                .iter()
                .find(|field| field.ident.as_ref().is_some_and(|name| name == method))
                .unwrap();
            let syn::Type::FnPtr(pointer) = &field.ty else {
                panic!("method field must be a function pointer");
            };
            assert_eq!(
                pointer.abi.as_ref().unwrap().name.as_ref().unwrap().value(),
                convention
            );
            let shim = file
                .items
                .iter()
                .find_map(|item| match item {
                    syn::Item::Fn(item) if item.sig.ident == shim => Some(item),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                shim.sig
                    .abi
                    .as_ref()
                    .unwrap()
                    .name
                    .as_ref()
                    .unwrap()
                    .value(),
                convention
            );
        }
    }
}

#[test]
fn method_convention_override_rejects_unsupported_values_and_duplicates() {
    for convention in [
        "Rust",
        "C-unwind",
        "rust-intrinsic",
        "vectorcall",
        "unknown",
    ] {
        let declaration: syn::ItemTrait = syn::parse2(quote! {
            unsafe trait IMixed {
                #[abi(convention = #convention)]
                fn explicit_method(&self) -> u32;
            }
        })
        .unwrap();
        assert!(
            super::expand_native(quote! { abi = c }, &declaration)
                .unwrap_err()
                .to_string()
                .contains("unsupported convention")
        );
    }
    let non_string: syn::ItemTrait = syn::parse_quote! {
        unsafe trait IMixed {
            #[abi(convention = 7)]
            fn explicit_method(&self) -> u32;
        }
    };
    assert!(
        super::expand_native(quote! { abi = c }, &non_string)
            .unwrap_err()
            .to_string()
            .contains("string literal")
    );
    let duplicate: syn::ItemTrait = syn::parse_quote! {
        unsafe trait IMixed {
            #[abi(convention = "C", convention = "system")]
            fn explicit_method(&self) -> u32;
        }
    };
    assert!(
        super::expand_native(quote! { abi = c }, &duplicate)
            .unwrap_err()
            .to_string()
            .contains("only once")
    );
}

#[test]
fn inline_layout_is_explicit_and_c_only() {
    let item: syn::ItemTrait =
        syn::parse_quote! { unsafe trait IInline { fn value(&self) -> u32; } };
    let native = super::expand_native(quote! { abi = c, layout = inline }, &item)
        .unwrap()
        .to_string();
    assert!(native.contains("VtableLayout :: Inline"));
    assert!(native.contains("type Storage = IInlineVtbl"));
    assert!(native.contains("* vtable"));
    let borrowed = super::expand_abi(quote! { abi = c, layout = inline }, &item)
        .unwrap()
        .to_string();
    assert!(borrowed.contains("VtableLayout :: Inline"));
    assert!(!borrowed.contains("CppInterface"));
    for abi in [quote! { cpp }, quote! { msvc }, quote! { itanium }] {
        assert!(
            super::expand_native(quote! { abi = #abi, layout = inline }, &item)
                .unwrap_err()
                .to_string()
                .contains("only with abi = c")
        );
    }
    assert!(
        super::expand(
            quote! { abi = com, iid = "00000000-0000-0000-C000-000000000046", layout = inline },
            &item,
        )
        .unwrap_err()
        .to_string()
        .contains("only with abi = c")
    );
}

#[test]
fn inline_layout_validates_layout_names_duplicates_and_base_contract() {
    let item: syn::ItemTrait =
        syn::parse_quote! { unsafe trait IInline { fn value(&self) -> u32; } };
    assert!(
        super::expand_native(quote! { abi = c, layout = embedded }, &item)
            .unwrap_err()
            .to_string()
            .contains("pointer or inline")
    );
    assert!(
        super::expand_native(quote! { abi = c, layout = inline, layout = pointer }, &item)
            .unwrap_err()
            .to_string()
            .contains("only once")
    );
    let pointer = super::expand_native(quote! { abi = c, layout = pointer }, &item)
        .unwrap()
        .to_string();
    assert!(pointer.contains("type Storage = :: cppvtable :: VtablePtr"));
    let derived = super::expand_native(quote! { abi = c, layout = inline, extends(IBase) }, &item)
        .unwrap()
        .to_string();
    assert!(derived.contains("derived and base interfaces must use the same vtable layout"));
    assert!(derived.contains("an inline interface must have at least one function-pointer slot"));
}
