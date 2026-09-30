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
        ("safe_aggregate", false),
        ("safe_indirect", false),
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
                        assert_eq!(
                            matches!(item.sig.safety, syn::Safety::Unsafe(_)),
                            is_unsafe,
                            "caller {name} must keep the declared safety"
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
            #[abi(aggregate)]
            fn safe_aggregate(&self) -> Aggregate;
            #[abi(hidden_return)]
            fn safe_indirect(&self) -> Aggregate;
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

fn abi_name(abi: Option<&syn::Abi>) -> String {
    abi.unwrap().name.as_ref().unwrap().value()
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
        // An x86-only convention applies on x86 and lowers to "C" in the second version.
        let expected: &[&str] =
            if ["cdecl", "stdcall", "fastcall", "thiscall"].contains(&convention) {
                &[convention, "C"]
            } else {
                &[convention]
            };
        let tables: Vec<&syn::ItemStruct> = file
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Struct(item) if item.ident == "IMixedVtbl" => Some(item),
                _ => None,
            })
            .collect();
        for (method, shim) in [
            ("explicit_method", "__cppvtable_imixed_slot_1"),
            ("explicit_aggregate", "__cppvtable_imixed_slot_2"),
        ] {
            let fields: Vec<String> = tables
                .iter()
                .map(|table| {
                    let field = table
                        .fields
                        .iter()
                        .find(|field| field.ident.as_ref().is_some_and(|name| name == method))
                        .unwrap();
                    let syn::Type::FnPtr(pointer) = &field.ty else {
                        panic!("method field must be a function pointer");
                    };
                    abi_name(pointer.abi.as_ref())
                })
                .collect();
            assert_eq!(fields, expected, "{method} with {convention}");
            let shims: Vec<String> = file
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Fn(item) if item.sig.ident == shim => {
                        Some(abi_name(item.sig.abi.as_ref()))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(shims, expected, "{shim} with {convention}");
        }
        for table in &tables {
            let field = table
                .fields
                .iter()
                .find(|field| {
                    field
                        .ident
                        .as_ref()
                        .is_some_and(|name| name == "default_method")
                })
                .unwrap();
            let syn::Type::FnPtr(pointer) = &field.ty else {
                panic!("method field must be a function pointer");
            };
            assert_eq!(abi_name(pointer.abi.as_ref()), "C");
        }
    }
}

#[test]
fn x86_only_overrides_split_every_configuration_that_admits_x86() {
    let declaration: syn::ItemTrait = syn::parse_quote! {
        unsafe trait ISystem {
            #[abi(convention = "stdcall")]
            fn value(&self) -> u32;
        }
    };
    let output = super::expand_native(quote! { abi = cpp }, &declaration)
        .unwrap()
        .to_string();
    // MSVC x86, MSVC other, Itanium x86, Itanium other, Windows GNU x86.
    assert_eq!(output.matches("struct ISystemVtbl").count(), 5);
    let com = super::expand(
        quote! { abi = com, iid = "00000000-0000-0000-C000-000000000046" },
        &declaration,
    )
    .unwrap()
    .to_string();
    assert_eq!(com.matches("struct ISystemVtbl").count(), 2);
    assert!(com.contains("not (target_arch = \"x86\")"));
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

#[test]
fn a_declaration_must_be_an_unsafe_trait() {
    let item: syn::ItemTrait = syn::parse_quote! { pub trait IPlain { fn value(&self) -> u32; } };
    for error in [
        super::expand_native(quote! { abi = c }, &item).unwrap_err(),
        super::expand_abi(quote! { abi = cpp }, &item).unwrap_err(),
        super::expand(
            quote! { abi = com, iid = "00000000-0000-0000-C000-000000000046" },
            &item,
        )
        .unwrap_err(),
    ] {
        assert!(error.to_string().contains("unsafe trait"));
    }
}

fn has_attribute(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

fn allows_deprecated(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("allow") && quote! { #attr }.to_string().contains("deprecated")
    })
}

#[test]
fn trait_attributes_go_to_the_interface_type() {
    let item: syn::ItemTrait = syn::parse_quote! {
        #[deprecated = "use IValue2"]
        #[must_use]
        pub unsafe trait IValue { fn value(&self) -> u32; }
    };
    let file: syn::File =
        syn::parse2(super::expand_native(quote! { abi = c, extends(IBase) }, &item).unwrap())
            .unwrap();
    let mut impls = 0;
    for item in &file.items {
        match item {
            syn::Item::Struct(item) if item.ident == "IValue" => {
                assert!(has_attribute(&item.attrs, "deprecated"));
                assert!(has_attribute(&item.attrs, "must_use"));
            }
            syn::Item::Struct(item) => assert!(!has_attribute(&item.attrs, "deprecated")),
            syn::Item::Impl(impl_item)
                if matches!(
                    impl_item.self_ty.as_ref(),
                    syn::Type::Path(ty) if ty.path.is_ident("IValue")
                ) =>
            {
                assert!(
                    allows_deprecated(&impl_item.attrs),
                    "{}",
                    quote! { #impl_item }
                );
                impls += 1;
            }
            _ => {}
        }
    }
    // Inherent, Interface, CppInterface, Deref, Debug, PartialEq, and Eq.
    assert_eq!(impls, 7);

    for (attribute, expected) in [
        (quote! { #[derive(Clone, Copy)] }, "cannot derive"),
        (quote! { #[repr(C)] }, "repr(transparent)"),
    ] {
        let item: syn::ItemTrait = syn::parse_quote! {
            #attribute
            pub unsafe trait IValue { fn value(&self) -> u32; }
        };
        let message = super::expand_native(quote! { abi = c }, &item)
            .unwrap_err()
            .to_string();
        assert!(message.contains(expected), "{message}");
    }
}

#[test]
fn a_trait_cfg_configures_every_generated_item() {
    let item: syn::ItemTrait = syn::parse_quote! {
        #[cfg(feature = "value")]
        pub unsafe trait IValue { fn value(&self) -> u32; }
    };
    for output in [
        super::expand_native(quote! { abi = cpp }, &item).unwrap(),
        super::expand_abi(quote! { abi = c }, &item).unwrap(),
    ] {
        let file: syn::File = syn::parse2(output).unwrap();
        for item in &file.items {
            let tokens = quote! { #item }.to_string();
            assert!(
                tokens.starts_with("# [cfg (feature = \"value\")]"),
                "{tokens}"
            );
        }
    }
}

#[test]
fn method_attributes_go_to_the_caller_and_the_implementation() {
    let item: syn::ItemTrait = syn::parse_quote! {
        pub unsafe trait IValue {
            #[deprecated]
            #[must_use]
            #[allow(clippy::pedantic, reason = "Test.")]
            fn value(&self) -> u32;
        }
    };
    let file: syn::File =
        syn::parse2(super::expand_native(quote! { abi = c }, &item).unwrap()).unwrap();
    let forwarded = |attrs: &[syn::Attribute]| {
        ["deprecated", "must_use", "allow"].map(|name| has_attribute(attrs, name))
    };
    let mut callers = 0;
    for item in &file.items {
        match item {
            syn::Item::Impl(item) => {
                for item in &item.items {
                    if let syn::ImplItem::Fn(item) = item {
                        if item.sig.ident == "value" {
                            assert_eq!(forwarded(&item.attrs), [true; 3]);
                            callers += 1;
                        }
                    }
                }
            }
            syn::Item::Trait(item) if item.ident == "IValueImpl" => {
                let syn::TraitItem::Fn(method) = &item.items[0] else {
                    panic!("the implementation trait holds the method");
                };
                assert_eq!(forwarded(&method.attrs), [true; 3]);
            }
            syn::Item::Fn(item) if item.sig.ident == "__cppvtable_ivalue_slot_0" => {
                assert!(allows_deprecated(&item.attrs));
            }
            _ => {}
        }
    }
    assert_eq!(callers, 1);

    let configured: syn::ItemTrait = syn::parse_quote! {
        pub unsafe trait IValue {
            #[cfg(windows)]
            fn value(&self) -> u32;
        }
    };
    assert!(
        super::expand_native(quote! { abi = c }, &configured)
            .unwrap_err()
            .to_string()
            .contains("shift the slots")
    );
}

fn inherent_methods(output: TokenStream, name: &str) -> Vec<syn::ImplItemFn> {
    let file: syn::File = syn::parse2(output).unwrap();
    file.items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Impl(item) if item.trait_.is_none() => Some(item),
            _ => None,
        })
        .filter(
            |item| matches!(item.self_ty.as_ref(), syn::Type::Path(ty) if ty.path.is_ident(name)),
        )
        .flat_map(|item| item.items)
        .filter_map(|item| match item {
            syn::ImplItem::Fn(item) => Some(item),
            _ => None,
        })
        .collect()
}

#[test]
fn the_interface_type_borrows_and_hooks_through_its_contract() {
    let item: syn::ItemTrait =
        syn::parse_quote! { pub unsafe trait IValue { fn value(&self) -> u32; } };
    let names = |output: TokenStream| -> Vec<String> {
        inherent_methods(output, "IValue")
            .iter()
            .map(|method| method.sig.ident.to_string())
            .collect()
    };
    let native = names(super::expand_native(quote! { abi = cpp }, &item).unwrap());
    assert_eq!(
        native
            .iter()
            .filter(|name| *name != "value")
            .collect::<Vec<_>>(),
        ["as_raw", "from_raw", "from_non_null", "vtable", "hook"]
    );
    for output in [
        super::expand_native(quote! { abi = c, layout = inline }, &item).unwrap(),
        super::expand_abi(quote! { abi = cpp }, &item).unwrap(),
        super::expand(
            quote! { abi = com, iid = "00000000-0000-0000-C000-000000000046" },
            &item,
        )
        .unwrap(),
    ] {
        assert!(!names(output).contains(&"hook".to_owned()));
    }

    let methods = inherent_methods(
        super::expand_native(quote! { abi = c }, &item).unwrap(),
        "IValue",
    );
    let signature = |name: &str| {
        let method = methods
            .iter()
            .find(|method| method.sig.ident == name)
            .unwrap();
        let signature = &method.sig;
        quote! { #signature }.to_string()
    };
    assert!(
        signature("from_raw").contains("Option < :: cppvtable :: InterfaceRef < 'a , Self > >")
    );
    assert!(signature("from_non_null").contains("-> :: cppvtable :: InterfaceRef < 'a , Self >"));
    assert!(signature("vtable").contains("-> & IValueVtbl"));
    assert!(!signature("vtable").contains("unsafe"));

    let abi = super::expand_abi(quote! { abi = c }, &item)
        .unwrap()
        .to_string();
    assert!(abi.contains("struct IValue (:: cppvtable_abi :: RawInterface) ;"));
    assert!(abi.contains("impl :: core :: fmt :: Debug for IValue"));
    assert!(abi.contains("\"IValue({:p})\""));
    assert!(abi.contains("impl :: core :: cmp :: Eq for IValue"));
}
