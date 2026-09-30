//! Validation of a declaration, and the helpers for the lint attributes.
//!
//! The generated code must be clean under a lint set that makes every warning an error
//! and that denies `#[allow]`. `#[expect]` is an error when the lint does not fire, so
//! the macro adds an expectation only when it knows that the lint fires. The functions
//! [`is_snake_case`] and [`needs_non_snake_case`] give that knowledge for
//! `non_snake_case`. The only generated `#[allow]` is `deprecated` with a reason, on the
//! items that implement a deprecated declaration, because whether a use inside an
//! implementation warns is not known in advance.

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{FnArg, ReceiverKind, ReturnType, Signature, Type};

/// The four fields of a GUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GuidParts {
    /// The first 8 hexadecimal digits.
    pub(crate) data1: u32,
    /// The first group of 4 hexadecimal digits.
    pub(crate) data2: u16,
    /// The second group of 4 hexadecimal digits.
    pub(crate) data3: u16,
    /// The last 8 bytes.
    pub(crate) data4: [u8; 8],
}

/// How a method gives its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReturnKind {
    /// The method has no return value.
    Unit,
    /// The value goes back in a register.
    Scalar,
    /// The value goes back through a hidden pointer after `this`.
    Hidden,
    /// A trivial repr(C) aggregate lowered according to the target C/C++ ABI.
    Aggregate,
}

/// The return types that the MSVC ABI gives back in a register.
const SCALAR_TYPES: &[&str] = &[
    "bool",
    "char",
    "f32",
    "f64",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    "c_char",
    "c_schar",
    "c_uchar",
    "c_short",
    "c_ushort",
    "c_int",
    "c_uint",
    "c_long",
    "c_ulong",
    "c_longlong",
    "c_ulonglong",
    "c_float",
    "c_double",
    // The names of the header of Windows. Each one is a transparent wrapper of a number
    // or of a pointer.
    "BOOL",
    "BOOLEAN",
    "BYTE",
    "CHAR",
    "DWORD",
    "FLOAT",
    "HRESULT",
    "INT",
    "LONG",
    "LONGLONG",
    "LPARAM",
    "LPVOID",
    "LRESULT",
    "NTSTATUS",
    "SHORT",
    "UCHAR",
    "UINT",
    "ULONG",
    "ULONGLONG",
    "USHORT",
    "WORD",
    "WPARAM",
];

/// The types that Rust owns. They have no stable binary layout.
const RUST_ONLY_TYPES: &[&str] = &[
    "Arc", "Box", "Cow", "HashMap", "HashSet", "Rc", "RefCell", "Result", "String", "Vec", "str",
];

/// Tell if a name obeys the rule of the lint `non_snake_case`.
///
/// The rule of the compiler is: remove the underscores at the start and at the end. The
/// rest must have no uppercase letter and no pair of underscores.
pub(crate) fn is_snake_case(name: &str) -> bool {
    let trimmed = name.trim_matches('_');
    let mut after_underscore = false;
    for character in trimmed.chars() {
        if character == '_' {
            if after_underscore {
                return false;
            }
            after_underscore = true;
        } else if character.is_uppercase() {
            return false;
        } else {
            after_underscore = false;
        }
    }
    true
}

/// Tell if the generated item needs `#[expect(non_snake_case)]`.
///
/// The answer is true when at least one name breaks the rule. The expectation then
/// always fires, so the compiler does not report an unfulfilled expectation.
pub(crate) fn needs_non_snake_case<'a>(names: impl IntoIterator<Item = &'a str>) -> bool {
    names.into_iter().any(|name| !is_snake_case(name))
}

/// Read a GUID from the text of the `iid` argument.
///
/// The format is `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`. Braces are permitted.
pub(crate) fn parse_guid(text: &str, span: Span) -> Result<GuidParts, syn::Error> {
    let body = text.trim().trim_start_matches('{').trim_end_matches('}');
    let groups: Vec<&str> = body.split('-').collect();
    let error = |message: &str| {
        syn::Error::new(
            span,
            format!(
                "iid: {message}. The format is \
                 \"xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx\"."
            ),
        )
    };
    if groups.len() != 5 {
        return Err(error("the value needs 5 groups that a hyphen separates"));
    }
    let lengths = [8_usize, 4, 4, 4, 12];
    for (group, length) in groups.iter().zip(lengths) {
        if group.len() != length || !group.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(error("a group has the wrong length or is not hexadecimal"));
        }
    }

    let parse_u32 = |text: &str| u32::from_str_radix(text, 16).map_err(|_| error("bad number"));
    let parse_u16 = |text: &str| u16::from_str_radix(text, 16).map_err(|_| error("bad number"));

    let data1 = parse_u32(groups[0])?;
    let data2 = parse_u16(groups[1])?;
    let data3 = parse_u16(groups[2])?;
    let mut data4 = [0_u8; 8];
    let tail: String = [groups[3], groups[4]].concat();
    for (index, byte) in data4.iter_mut().enumerate() {
        let pair = tail
            .get(index * 2..index * 2 + 2)
            .ok_or_else(|| error("the last groups are too short"))?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| error("bad number"))?;
    }
    Ok(GuidParts {
        data1,
        data2,
        data3,
        data4,
    })
}

/// Check the signature of a method of an interface declaration.
pub(crate) fn check_signature(signature: &Signature) -> Result<(), syn::Error> {
    let name = &signature.ident;
    if signature.asyncness.is_some() {
        return Err(syn::Error::new(
            name.span(),
            format!("method `{name}`: an async method has no vtable slot"),
        ));
    }
    if signature.constness.is_some() {
        return Err(syn::Error::new(
            name.span(),
            format!("method `{name}`: a const method has no vtable slot"),
        ));
    }
    if !signature.generics.params.is_empty() {
        return Err(syn::Error::new(
            signature.generics.span(),
            format!("method `{name}`: a generic method has no vtable slot"),
        ));
    }
    if signature.variadic.is_some() {
        return Err(syn::Error::new(
            name.span(),
            format!("method `{name}`: a variadic method is not supported"),
        ));
    }
    if signature.abi.is_some() {
        return Err(syn::Error::new(
            name.span(),
            format!(
                "method `{name}`: do not give a calling convention. The `abi` argument \
                 of `#[interface]` gives it."
            ),
        ));
    }

    let Some(receiver) = signature.receiver() else {
        return Err(syn::Error::new(
            name.span(),
            format!("method `{name}`: the first argument must be `&self`"),
        ));
    };
    match &receiver.kind {
        ReceiverKind::Reference(_, _, mutability) => {
            if mutability.is_some() {
                return Err(syn::Error::new(
                    receiver.self_token.span(),
                    format!(
                        "method `{name}`: use `&self`. A foreign caller can call the \
                         object again during the call and from another thread, so a \
                         `&mut self` is undefined behaviour. Use interior mutability."
                    ),
                ));
            }
        }
        _ => {
            return Err(syn::Error::new(
                receiver.self_token.span(),
                format!("method `{name}`: use `&self`"),
            ));
        }
    }

    for argument in &signature.inputs {
        if let FnArg::Typed(typed) = argument {
            if let Err(message) = check_ffi_type(&typed.ty) {
                return Err(syn::Error::new(
                    typed.ty.span(),
                    format!("method `{name}`: {message}"),
                ));
            }
        }
    }
    Ok(())
}

/// Decide how the method gives its answer.
///
/// `scalar` and `hidden` come from `#[abi(scalar)]` and `#[abi(hidden_return)]`.
pub(crate) fn classify_return(
    output: &ReturnType,
    name: &syn::Ident,
    scalar: bool,
    hidden: bool,
) -> Result<ReturnKind, syn::Error> {
    let ReturnType::Type(_, ty) = output else {
        if hidden {
            return Err(syn::Error::new(
                name.span(),
                format!("method `{name}`: `#[abi(hidden_return)]` needs a return type"),
            ));
        }
        return Ok(ReturnKind::Unit);
    };
    if let Err(message) = check_ffi_type(ty) {
        return Err(syn::Error::new(
            ty.span(),
            format!("method `{name}`: return type: {message}"),
        ));
    }
    if hidden {
        if scalar {
            return Err(syn::Error::new(
                name.span(),
                format!(
                    "method `{name}`: use `#[abi(scalar)]` or `#[abi(hidden_return)]`, \
                     not both"
                ),
            ));
        }
        return Ok(ReturnKind::Hidden);
    }
    if scalar || is_scalar_type(ty) {
        return Ok(ReturnKind::Scalar);
    }
    Err(syn::Error::new(
        ty.span(),
        format!(
            "method `{name}`: this return type is not a number and not a pointer. MSVC \
             gives a structure back through a hidden pointer after `this`, also when the \
             structure is small, but a Rust `extern` function does not. Add \
             `#[abi(hidden_return)]` to the method to make that shim, or add \
             `#[abi(scalar)]` when the type is a transparent wrapper of a number or of a \
             pointer."
        ),
    ))
}

/// Tell if the MSVC ABI gives the type back in a register.
fn is_scalar_type(ty: &Type) -> bool {
    match ty {
        Type::Ptr(_) | Type::FnPtr(_) | Type::Never(_) => true,
        Type::Tuple(tuple) => tuple.elems.is_empty(),
        Type::Path(path) => {
            path.qself.is_none()
                && last_name(ty).is_some_and(|name| SCALAR_TYPES.contains(&name.as_str()))
        }
        _ => false,
    }
}

/// Give the name of the last segment of a path type.
fn last_name(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else { return None };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

/// Tell if a type has a stable binary layout.
fn check_ffi_type(ty: &Type) -> Result<(), String> {
    match ty {
        Type::Reference(reference) => {
            let kind = if reference.mutability.is_some() {
                "&mut T"
            } else {
                "&T"
            };
            Err(format!(
                "`{kind}` is not permitted here. A reference has rules that foreign code \
                 does not obey. Use `*const T` or `*mut T`."
            ))
        }
        Type::Slice(_) => Err("a slice has no stable layout. Use a pointer and a length.".into()),
        Type::TraitObject(_) => Err("a trait object has no stable layout.".into()),
        Type::ImplTrait(_) => Err("`impl Trait` has no stable layout.".into()),
        Type::Tuple(tuple) if !tuple.elems.is_empty() => {
            Err("a tuple has no stable layout. Use a `#[repr(C)]` structure.".into())
        }
        Type::Path(_) => match last_name(ty) {
            Some(name) if RUST_ONLY_TYPES.contains(&name.as_str()) => Err(format!(
                "`{name}` is a type of Rust and has no stable layout. Use a pointer."
            )),
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{ReturnKind, classify_return, is_snake_case, needs_non_snake_case, parse_guid};
    use proc_macro2::Span;
    use quote::format_ident;

    #[test]
    fn snake_case_follows_the_rule_of_the_compiler() {
        assert!(is_snake_case("lock"));
        assert!(is_snake_case("get_desc"));
        assert!(is_snake_case("__reserved_3"));
        assert!(is_snake_case("_"));
        assert!(is_snake_case(""));
        assert!(!is_snake_case("Lock"));
        assert!(!is_snake_case("GetDesc"));
        assert!(!is_snake_case("get__desc"));
        assert!(!is_snake_case("queryInterface"));
    }

    #[test]
    fn the_expectation_comes_only_with_a_name_that_breaks_the_rule() {
        assert!(needs_non_snake_case(["lock", "GetDesc"]));
        assert!(!needs_non_snake_case(["lock", "unlock"]));
        assert!(!needs_non_snake_case([]));
    }

    #[test]
    fn a_guid_reads_correctly() {
        let guid = parse_guid("d0223b96-bf7a-43fd-92bd-a43b0d82b9eb", Span::call_site()).unwrap();
        assert_eq!(guid.data1, 0xd022_3b96);
        assert_eq!(guid.data2, 0xbf7a);
        assert_eq!(guid.data3, 0x43fd);
        assert_eq!(guid.data4, [0x92, 0xbd, 0xa4, 0x3b, 0x0d, 0x82, 0xb9, 0xeb]);

        let braces =
            parse_guid("{00000000-0000-0000-C000-000000000046}", Span::call_site()).unwrap();
        assert_eq!(braces.data1, 0);
        assert_eq!(braces.data4, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
    }

    #[test]
    fn a_bad_guid_gives_an_error() {
        assert!(parse_guid("not-a-guid", Span::call_site()).is_err());
        assert!(parse_guid("d0223b96-bf7a-43fd-92bd", Span::call_site()).is_err());
        assert!(parse_guid("d0223b96-bf7a-43fd-92bd-a43b0d82b9ez", Span::call_site()).is_err());
    }

    #[test]
    fn the_return_type_rules_are_correct() {
        let name = format_ident!("GetDesc");
        let scalar: syn::ReturnType = syn::parse_quote!(-> u32);
        let hresult: syn::ReturnType = syn::parse_quote!(-> HRESULT);
        let pointer: syn::ReturnType = syn::parse_quote!(-> *mut c_void);
        let unit: syn::ReturnType = syn::ReturnType::Default;
        let aggregate: syn::ReturnType = syn::parse_quote!(-> D3DVECTOR);
        let rust_only: syn::ReturnType = syn::parse_quote!(-> String);

        assert_eq!(
            classify_return(&scalar, &name, false, false).unwrap(),
            ReturnKind::Scalar
        );
        assert_eq!(
            classify_return(&hresult, &name, false, false).unwrap(),
            ReturnKind::Scalar
        );
        assert_eq!(
            classify_return(&pointer, &name, false, false).unwrap(),
            ReturnKind::Scalar
        );
        assert_eq!(
            classify_return(&unit, &name, false, false).unwrap(),
            ReturnKind::Unit
        );
        assert!(classify_return(&aggregate, &name, false, false).is_err());
        assert_eq!(
            classify_return(&aggregate, &name, true, false).unwrap(),
            ReturnKind::Scalar
        );
        assert_eq!(
            classify_return(&aggregate, &name, false, true).unwrap(),
            ReturnKind::Hidden
        );
        assert!(classify_return(&aggregate, &name, true, true).is_err());
        assert!(classify_return(&rust_only, &name, false, true).is_err());
        assert!(classify_return(&unit, &name, false, true).is_err());
    }

    #[test]
    fn a_bad_signature_gives_an_error() {
        use super::check_signature;
        let good: syn::TraitItemFn = syn::parse_quote!(
            fn Lock(&self, offset: u32) -> HRESULT;
        );
        assert!(check_signature(&good.sig).is_ok());

        let by_value: syn::TraitItemFn = syn::parse_quote!(
            fn Lock(self) -> HRESULT;
        );
        assert!(check_signature(&by_value.sig).is_err());

        let mutable: syn::TraitItemFn = syn::parse_quote!(
            fn Lock(&mut self) -> HRESULT;
        );
        assert!(check_signature(&mutable.sig).is_err());

        let no_self: syn::TraitItemFn = syn::parse_quote!(
            fn Lock(offset: u32) -> HRESULT;
        );
        assert!(check_signature(&no_self.sig).is_err());

        let generic: syn::TraitItemFn = syn::parse_quote!(
            fn Lock<T>(&self, value: T) -> HRESULT;
        );
        assert!(check_signature(&generic.sig).is_err());

        let reference: syn::TraitItemFn = syn::parse_quote!(
            fn Lock(&self, value: &u32) -> HRESULT;
        );
        assert!(check_signature(&reference.sig).is_err());

        let owned: syn::TraitItemFn = syn::parse_quote!(
            fn Lock(&self, value: String) -> HRESULT;
        );
        assert!(check_signature(&owned.sig).is_err());
    }
}
