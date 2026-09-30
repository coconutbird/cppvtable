//! The code generator of `#[vtable_fn]`.
//!
//! The attribute gives a free `unsafe fn` the calling convention that the vtable fields
//! of one binary interface use on every target. It emits one copy of the function for
//! each version of that interface's vtable, each under the same `cfg` as the matching
//! vtable structure, so the function coerces to the field type everywhere.

use proc_macro2::TokenStream;
use quote::quote;
use syn::ItemFn;
use syn::spanned::Spanned;

use crate::abi::is_x86_only;
use crate::parse::VtableFnArgs;

/// Expand `#[vtable_fn(...)]`.
pub(crate) fn expand(args: TokenStream, item: &ItemFn) -> Result<TokenStream, syn::Error> {
    let args = VtableFnArgs::parse(args)?;
    let name = &item.sig.ident;
    if !matches!(item.sig.safety, syn::Safety::Unsafe(_)) {
        return Err(syn::Error::new(
            name.span(),
            "declare the function as an `unsafe fn`: vtable entries are `unsafe extern` \
             function pointers whose `this` argument is a raw pointer",
        ));
    }
    if let Some(abi) = &item.sig.abi {
        return Err(syn::Error::new(
            abi.span(),
            "do not give a calling convention. The `abi` argument of `#[vtable_fn]` gives it.",
        ));
    }
    if item.sig.asyncness.is_some() {
        return Err(syn::Error::new(
            name.span(),
            "an async function cannot be a vtable entry",
        ));
    }

    let split_x86 = args
        .convention
        .as_ref()
        .is_some_and(|convention| is_x86_only(&convention.value()));
    let mut output = args.abi.target_guard();
    for variant in args.abi.variants(split_x86) {
        let mut copy = item.clone();
        copy.sig.abi = Some(syn::Abi {
            extern_token: <syn::Token![extern]>::default(),
            name: Some(variant.convention_for(args.convention.as_ref())),
        });
        let cfg = variant.cfg();
        output.extend(quote! {
            #cfg
            #copy
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use proc_macro2::TokenStream;
    use quote::quote;

    fn expand(args: TokenStream, item: TokenStream) -> Result<syn::File, String> {
        let parsed: syn::ItemFn = syn::parse2(item).unwrap();
        super::expand(args, &parsed)
            .map(|tokens| syn::parse2(tokens).unwrap())
            .map_err(|error| error.to_string())
    }

    fn conventions(file: &syn::File) -> Vec<String> {
        file.items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Fn(function) => Some(
                    function
                        .sig
                        .abi
                        .as_ref()
                        .unwrap()
                        .name
                        .as_ref()
                        .unwrap()
                        .value(),
                ),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn each_vtable_version_gets_a_copy_with_its_convention() {
        let item = quote! {
            unsafe fn entry(this: *mut ::core::ffi::c_void) -> u32 { 7 }
        };
        let cpp = expand(quote! { abi = cpp }, item.clone()).unwrap();
        assert_eq!(conventions(&cpp), ["thiscall", "C", "C", "thiscall"]);
        let com = expand(quote! { abi = com }, item.clone()).unwrap();
        assert_eq!(conventions(&com), ["system"]);
        let c = expand(quote! { abi = c }, item.clone()).unwrap();
        assert_eq!(conventions(&c), ["C"]);
        let msvc = expand(quote! { abi = msvc }, item.clone()).unwrap();
        assert!(quote! { #msvc }.to_string().contains("compile_error"));
        assert_eq!(conventions(&msvc), ["thiscall", "C"]);

        let stdcall = expand(quote! { abi = c, convention = "stdcall" }, item.clone()).unwrap();
        assert_eq!(conventions(&stdcall), ["stdcall", "C"]);
        let win64 = expand(quote! { abi = c, convention = "win64" }, item).unwrap();
        assert_eq!(conventions(&win64), ["win64"]);
    }

    #[test]
    fn a_bad_function_or_argument_gives_a_clear_error() {
        let safe = expand(quote! { abi = c }, quote! { fn entry(this: *mut u8) {} });
        assert!(safe.unwrap_err().contains("unsafe fn"));
        let explicit = expand(
            quote! { abi = c },
            quote! { unsafe extern "C" fn entry(this: *mut u8) {} },
        );
        assert!(
            explicit
                .unwrap_err()
                .contains("do not give a calling convention")
        );
        let missing = expand(quote! {}, quote! { unsafe fn entry(this: *mut u8) {} });
        assert!(missing.unwrap_err().contains("give the binary interface"));
        let unknown = expand(
            quote! { abi = rust },
            quote! { unsafe fn entry(this: *mut u8) {} },
        );
        assert!(unknown.unwrap_err().contains("`rust` is unknown"));
        let bad_convention = expand(
            quote! { abi = c, convention = "vectorcall" },
            quote! { unsafe fn entry(this: *mut u8) {} },
        );
        assert!(
            bad_convention
                .unwrap_err()
                .contains("unsupported convention")
        );
    }
}
