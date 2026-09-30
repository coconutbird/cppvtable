//! Tests of the expansion of `#[interface]`.
//!
//! The tests read the generated tokens as text. They check the names of the generated
//! items, the calling conventions, and the lint attributes. They also check that each
//! rule of the declaration gives a clear error.

use proc_macro2::TokenStream;
use quote::quote;

/// Expand a declaration that must be correct and give the tokens as text.
fn expand_ok(args: TokenStream, item: TokenStream) -> String {
    let parsed: syn::ItemTrait = syn::parse2(item).unwrap();
    super::expand(args, &parsed).unwrap().to_string()
}

/// Expand a declaration that must fail and give the message.
fn expand_err(args: TokenStream, item: TokenStream) -> String {
    let parsed: syn::ItemTrait = syn::parse2(item).unwrap();
    super::expand(args, &parsed).unwrap_err().to_string()
}

/// A small COM declaration with names of a C header.
fn com_trait() -> TokenStream {
    quote! {
        pub unsafe trait IFoo {
            /// Give the value.
            fn GetValue(&self, value: *mut u32) -> HRESULT;
            fn Release2(&self) -> u32;
        }
    }
}

#[test]
fn the_macro_makes_every_item_of_the_interface() {
    let text = expand_ok(
        quote! { abi = com, iid = "00112233-4455-6677-8899-aabbccddeeff" },
        com_trait(),
    );
    assert!(text.contains("pub struct IFoo"));
    assert!(text.contains("pub struct IFooVtbl"));
    assert!(text.contains("pub trait IFooImpl"));
    assert!(text.contains("impl :: cppvtable :: Interface for IFoo"));
    assert!(text.contains("const fn new < T : IFooImpl , const SLOT : usize >"));
    assert!(text.contains("__cppvtable_ifoo_slot_0"));
    assert!(text.contains("__cppvtable_ifoo_slot_1"));
    // The base of a COM interface without `extends` is `IUnknown`.
    assert!(text.contains("pub base : :: cppvtable :: IUnknownVtbl"));
    assert!(text.contains(":: cppvtable :: IUnknownVtbl :: new :: < T , SLOT > ()"));
    assert!(text.contains("impl :: core :: ops :: Deref for IFoo"));
}

#[test]
fn the_calling_convention_follows_the_abi_argument() {
    let com = expand_ok(
        quote! { abi = com, iid = "00112233-4455-6677-8899-aabbccddeeff" },
        com_trait(),
    );
    assert!(com.contains("extern \"system\""));
    assert!(!com.contains("target_arch"));

    let plain_c = expand_ok(
        quote! { abi = c },
        quote! {
            pub unsafe trait ITable {
                fn get(&self) -> u32;
            }
        },
    );
    assert!(plain_c.contains("extern \"C\""));
    assert!(!plain_c.contains("target_arch"));

    let cpp = expand_ok(
        quote! { abi = cpp },
        quote! {
            pub unsafe trait IShape {
                fn area(&self) -> u32;
            }
        },
    );
    // A `cpp` interface needs one version for x86 and one for every other target.
    assert!(cpp.contains("extern \"thiscall\""));
    assert!(cpp.contains("cfg (target_arch = \"x86\")"));
    assert!(cpp.contains("cfg (not (target_arch = \"x86\"))"));
    // A `cpp` vtable has no `IUnknown` part.
    assert!(!cpp.contains("IUnknownVtbl"));
}

#[test]
fn the_expectation_of_non_snake_case_comes_only_with_a_name_of_a_c_header() {
    let com = expand_ok(
        quote! { abi = com, iid = "00112233-4455-6677-8899-aabbccddeeff" },
        com_trait(),
    );
    assert!(com.contains("expect (non_snake_case"));

    let snake = expand_ok(
        quote! { abi = c },
        quote! {
            pub unsafe trait ITable {
                fn get_value(&self) -> u32;
            }
        },
    );
    // An expectation that never fires is an error, so the macro must not add one here.
    assert!(!snake.contains("non_snake_case"));
}

#[test]
fn the_expectation_of_too_many_arguments_comes_only_with_a_long_signature() {
    let short = expand_ok(
        quote! { abi = c },
        quote! {
            pub unsafe trait ITable {
                fn six(&self, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32);
            }
        },
    );
    assert!(!short.contains("too_many_arguments"));

    let long = expand_ok(
        quote! { abi = c },
        quote! {
            pub unsafe trait ITable {
                fn eight(&self, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, g: u32, h: u32);
            }
        },
    );
    assert!(long.contains("clippy :: too_many_arguments"));
}

#[test]
fn an_explicit_slot_makes_reserved_entries() {
    let text = expand_ok(
        quote! { abi = c },
        quote! {
            pub unsafe trait ITable {
                fn first(&self);
                #[slot(3)]
                fn fourth(&self);
            }
        },
    );
    assert!(text.contains("pub reserved_1 :"));
    assert!(text.contains("pub reserved_2 :"));
    assert!(text.contains("reserved_1 : :: core :: option :: Option :: None"));
    assert!(text.contains("__cppvtable_itable_slot_3"));
}

#[test]
fn a_hidden_return_makes_the_shim_of_the_msvc_rule() {
    let text = expand_ok(
        quote! { abi = c },
        quote! {
            pub unsafe trait ITable {
                #[abi(hidden_return)]
                fn get_box(&self) -> BoundingBox;
            }
        },
    );
    assert!(text.contains("result : * mut BoundingBox"));
    assert!(text.contains("-> * mut BoundingBox"));
    assert!(text.contains(":: core :: ptr :: write (result , value)"));
    assert!(text.contains("MaybeUninit :: < BoundingBox > :: uninit ()"));
}

#[test]
fn the_internal_flag_changes_the_paths() {
    let text = expand_ok(
        quote! { abi = com, iid = "00112233-4455-6677-8899-aabbccddeeff", internal },
        com_trait(),
    );
    assert!(text.contains("impl crate :: Interface for IFoo"));
    assert!(!text.contains(":: cppvtable ::"));
}

#[test]
fn the_root_flag_leaves_out_the_impl_trait_and_the_builder() {
    let text = expand_ok(
        quote! { abi = com, iid = "00000000-0000-0000-C000-000000000046", root, internal },
        quote! {
            pub unsafe trait IUnknown {
                fn QueryInterface(&self, riid: *const GUID, out: *mut *mut c_void) -> HRESULT;
            }
        },
    );
    assert!(text.contains("pub struct IUnknownVtbl"));
    assert!(!text.contains("trait IUnknownImpl"));
    assert!(!text.contains("const fn new"));
    assert!(text.contains("const ANCESTORS : & 'static [crate :: GUID] = & []"));
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
        let message = expand_err(quote! { abi = c }, item);
        assert!(
            message.contains(expected),
            "the message `{message}` must hold `{expected}`"
        );
    }
}

#[test]
fn a_slot_that_is_already_in_use_gives_an_error() {
    let message = expand_err(
        quote! { abi = c },
        quote! {
            pub unsafe trait ITable {
                fn first(&self);
                fn second(&self);
                #[slot(1)]
                fn third(&self);
            }
        },
    );
    assert!(message.contains("slot(1) is already in use"));
    assert!(message.contains("next free slot of this interface is 2"));
}
