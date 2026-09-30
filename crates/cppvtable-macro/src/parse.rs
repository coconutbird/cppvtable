//! The model of a declaration, and the readers of the macro arguments.

use proc_macro2::{Ident, Span, TokenStream};
use quote::format_ident;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Expr, ExprLit, FnArg, ItemTrait, Lit, Meta, Pat, Path, ReturnType, Token, TraitItem,
    Type,
};

use crate::abi::Abi;
use crate::validate::{GuidParts, ReturnKind, check_signature, classify_return, parse_guid};

/// Stable foreign conventions accepted by per-method overrides.
const CONVENTIONS: &[&str] = &[
    "C", "system", "cdecl", "stdcall", "fastcall", "thiscall", "win64", "sysv64", "aapcs",
];

/// Whether an interface pointer addresses a vtable pointer or the function table itself.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Layout {
    #[default]
    Pointer,
    Inline,
}

impl Layout {
    fn parse(value: &Expr) -> Result<Self, syn::Error> {
        match path_expr_name(value).as_deref() {
            Some("pointer") => Ok(Self::Pointer),
            Some("inline") => Ok(Self::Inline),
            _ => Err(syn::Error::new(
                value.span(),
                "layout: use pointer or inline",
            )),
        }
    }

    pub(crate) fn validate(self, abi: Abi, span: Span) -> Result<(), syn::Error> {
        if self == Self::Inline && abi != Abi::C {
            return Err(syn::Error::new(
                span,
                "layout = inline is supported only with abi = c",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Runtime {
    Abi,
    Native,
    Com,
}

/// The arguments of `#[interface(...)]`.
pub(crate) struct InterfaceArgs {
    /// The binary interface.
    pub(crate) abi: Abi,
    /// Pointer indirection or a function table stored directly in the object.
    pub(crate) layout: Layout,
    /// The interface identifier. A `cpp` or `c` interface may have none.
    pub(crate) iid: Option<GuidParts>,
    /// The base interface of the chain.
    pub(crate) extends: Option<Path>,
    /// The interface has no base and the crate supplies the vtable builder. Only
    /// `IUnknown` uses this.
    pub(crate) root: bool,
    /// Total vtable pointer slots, including any base-interface prefix.
    pub(crate) slots: Option<usize>,
    /// The declaration is inside a runtime crate, so generated paths start with `crate`.
    pub(crate) internal: bool,
}

/// Parse the COM-only interface identifier argument.
fn parse_iid_argument(value: &Expr) -> Result<GuidParts, syn::Error> {
    let Expr::Lit(ExprLit {
        lit: Lit::Str(text),
        ..
    }) = value
    else {
        return Err(syn::Error::new(
            value.span(),
            "iid: give the value as a string literal",
        ));
    };
    parse_guid(&text.value(), text.span())
}

/// Parse an explicit total vtable extent.
fn parse_slot_extent(value: &Expr) -> Result<usize, syn::Error> {
    let Expr::Lit(ExprLit {
        lit: Lit::Int(integer),
        ..
    }) = value
    else {
        return Err(syn::Error::new(
            value.span(),
            "slots requires a nonnegative integer literal",
        ));
    };
    integer.base10_parse()
}

impl InterfaceArgs {
    /// Read the arguments.
    pub(crate) fn parse(tokens: TokenStream) -> Result<Self, syn::Error> {
        let span = tokens.span();
        let items = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
        let mut abi = None;
        let mut iid = None;
        let mut extends = None;
        let mut root = false;
        let mut internal = false;
        let mut slots = None;
        let mut layout = None;

        for item in &items {
            match item {
                Meta::NameValue(pair) if pair.path.is_ident("abi") => {
                    abi = Some(parse_abi(&pair.value)?);
                }
                Meta::NameValue(pair) if pair.path.is_ident("layout") => {
                    if layout.is_some() {
                        return Err(syn::Error::new(
                            pair.span(),
                            "layout may be specified only once",
                        ));
                    }
                    layout = Some(Layout::parse(&pair.value)?);
                }
                Meta::NameValue(pair) if pair.path.is_ident("iid") => {
                    iid = Some(parse_iid_argument(&pair.value)?);
                }
                Meta::NameValue(pair) if pair.path.is_ident("slots") => {
                    if slots.is_some() {
                        return Err(syn::Error::new(
                            pair.span(),
                            "slots may be specified only once",
                        ));
                    }
                    slots = Some(parse_slot_extent(&pair.value)?);
                }
                Meta::List(list) if list.path.is_ident("extends") => {
                    extends = Some(syn::parse2::<Path>(list.tokens.clone())?);
                }
                Meta::Path(path) if path.is_ident("root") => root = true,
                Meta::Path(path) if path.is_ident("internal") => internal = true,
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "unknown argument. Use `abi = com|cpp|msvc|itanium|c`, `iid = \"...\"`, \
                         `layout = pointer|inline`, `extends(IBase)`, `slots = N`, `root`, or `internal`.",
                    ));
                }
            }
        }

        let abi = abi.ok_or_else(|| {
            syn::Error::new(
                span,
                "give the binary interface: `abi = com`, `cpp`, or `c`",
            )
        })?;
        let layout = layout.unwrap_or_default();
        layout.validate(abi, span)?;
        if abi.is_com() && iid.is_none() {
            return Err(syn::Error::new(
                span,
                "a COM interface needs `iid = \"...\"`",
            ));
        }
        if !abi.is_com() && iid.is_some() {
            return Err(syn::Error::new(
                span,
                "iid is COM metadata; C/C++ interfaces use type identity",
            ));
        }
        if root && extends.is_some() {
            return Err(syn::Error::new(
                span,
                "`root` and `extends` do not go together",
            ));
        }
        Ok(Self {
            abi,
            layout,
            iid,
            extends,
            root,
            slots,
            internal,
        })
    }

    /// Give the public runtime path for this macro entry point.
    pub(crate) fn krate(&self, runtime: Runtime) -> TokenStream {
        if self.internal {
            quote::quote! { crate }
        } else {
            match runtime {
                Runtime::Abi => quote::quote! { ::cppvtable_abi },
                Runtime::Native => quote::quote! { ::cppvtable },
                Runtime::Com => quote::quote! { ::cppvtable_com },
            }
        }
    }

    /// Give the ABI runtime path used in generated interface wrappers.
    pub(crate) fn abi_krate(runtime: Runtime) -> TokenStream {
        match runtime {
            Runtime::Abi => quote::quote! { ::cppvtable_abi },
            Runtime::Com => quote::quote! { ::cppvtable_com },
            Runtime::Native => quote::quote! { ::cppvtable },
        }
    }
}

/// One argument of a method.
pub(crate) struct Param {
    /// The name of the argument.
    pub(crate) name: Ident,
    /// The type of the argument.
    pub(crate) ty: Type,
}

/// One method of an interface.
pub(crate) struct Method {
    /// The name, with the spelling of the foreign header.
    pub(crate) name: Ident,
    /// The documentation of the declaration.
    pub(crate) docs: Vec<Attribute>,
    /// `deprecated`, `must_use`, `allow`, and `expect`, forwarded to the caller and the
    /// implementation method.
    pub(crate) attrs: Vec<Attribute>,
    /// The method carries a deprecation, so the generated shim must not warn when it
    /// calls the implementation.
    pub(crate) deprecated: bool,
    /// Declared Rust call contract, preserved on the implementation trait method.
    pub(crate) unsafety: Option<Token![unsafe]>,
    /// The arguments after `&self`.
    pub(crate) params: Vec<Param>,
    /// The return type of the declaration.
    pub(crate) output: ReturnType,
    /// Explicit foreign calling convention, otherwise the interface's default.
    pub(crate) convention: Option<syn::LitStr>,
    /// How the method gives its answer.
    pub(crate) kind: ReturnKind,
}

/// One entry of the derived part of the vtable.
pub(crate) struct Slot {
    /// The index inside the derived part.
    pub(crate) index: usize,
    /// The method. `None` is a reserved entry that `#[slot(N)]` made.
    pub(crate) method: Option<Method>,
}

/// The declaration of an interface.
pub(crate) struct InterfaceModel {
    /// The visibility of the declaration.
    pub(crate) vis: syn::Visibility,
    /// The name of the interface.
    pub(crate) name: Ident,
    /// The documentation of the declaration.
    pub(crate) docs: Vec<Attribute>,
    /// The other outer attributes, forwarded to the interface type.
    pub(crate) attrs: Vec<Attribute>,
    /// The `#[cfg]` attributes of the declaration, applied to every generated item.
    ///
    /// A `#[cfg]` before `#[interface]` is normally evaluated and removed first, but
    /// older compilers still pass the true predicate to the macro, and one written
    /// after `#[interface]` always arrives unevaluated.
    pub(crate) cfgs: Vec<Attribute>,
    /// The interface type carries a deprecation, so the generated items that name it
    /// must not warn.
    pub(crate) deprecated: bool,
    /// The entries of the derived part of the vtable, in slot order.
    pub(crate) slots: Vec<Slot>,
}

impl InterfaceModel {
    /// Read the trait declaration.
    pub(crate) fn parse(item: &ItemTrait) -> Result<Self, syn::Error> {
        if !item.generics.params.is_empty() {
            return Err(syn::Error::new(
                item.generics.span(),
                "an interface must not have generic parameters",
            ));
        }
        if item.unsafety.is_none() {
            return Err(syn::Error::new(
                item.trait_token.span(),
                "declare the interface as an `unsafe trait`. The `unsafe` promises that the \
                 slot order, signatures, calling conventions, and return lowering match the \
                 foreign header, that every method declared as a safe `fn` has no \
                 precondition beyond a live object, and that no method unwinds.",
            ));
        }
        if !item.supertraits.is_empty() {
            return Err(syn::Error::new(
                item.supertraits.span(),
                "use `extends(IBase)` in `#[interface]`, not a supertrait",
            ));
        }
        let (attrs, cfgs) = forwarded_trait_attributes(&item.attrs)?;

        let mut slots: Vec<Slot> = Vec::new();
        let mut next = 0_usize;
        for entry in &item.items {
            let TraitItem::Fn(function) = entry else {
                return Err(syn::Error::new(
                    entry.span(),
                    "an interface holds methods only",
                ));
            };
            if function.default.is_some() {
                return Err(syn::Error::new(
                    function.sig.ident.span(),
                    "a method of an interface has no body",
                ));
            }
            let options = MethodOptions::parse(&function.attrs)?;
            check_signature(&function.sig)?;
            let kind = options.return_kind(&function.sig.output, &function.sig.ident)?;

            let index = match options.slot {
                Some(explicit) => {
                    if explicit < next {
                        return Err(syn::Error::new(
                            function.sig.ident.span(),
                            format!(
                                "slot({explicit}) is already in use. The next free slot \
                                 of this interface is {next}."
                            ),
                        ));
                    }
                    for filler in next..explicit {
                        slots.push(Slot {
                            index: filler,
                            method: None,
                        });
                    }
                    explicit
                }
                None => next,
            };
            next = index + 1;

            let params = method_params(&function.sig);

            slots.push(Slot {
                index,
                method: Some(Method {
                    name: function.sig.ident.clone(),
                    docs: doc_attributes(&function.attrs),
                    deprecated: options.forwarded.iter().any(is_deprecation),
                    attrs: options.forwarded,
                    unsafety: match &function.sig.safety {
                        syn::Safety::Unsafe(token) => Some(*token),
                        _ => None,
                    },
                    params,
                    output: function.sig.output.clone(),
                    convention: options.convention,
                    kind,
                }),
            });
        }

        Ok(Self {
            vis: item.vis.clone(),
            name: item.ident.clone(),
            docs: doc_attributes(&item.attrs),
            deprecated: attrs.iter().any(is_deprecation),
            attrs,
            cfgs,
            slots,
        })
    }

    /// Give the names of the methods.
    pub(crate) fn method_names(&self) -> Vec<String> {
        self.slots
            .iter()
            .filter_map(|slot| slot.method.as_ref())
            .map(|method| method.name.to_string())
            .collect()
    }

    /// Tell if a method overrides its convention with one that exists only on x86.
    pub(crate) fn uses_x86_conventions(&self) -> bool {
        self.slots
            .iter()
            .filter_map(|slot| slot.method.as_ref()?.convention.as_ref())
            .any(|convention| crate::abi::is_x86_only(&convention.value()))
    }
}

/// The options that the attributes of a method give.
struct MethodOptions {
    /// The value of `#[slot(N)]`.
    slot: Option<usize>,
    /// `#[abi(scalar)]` is present.
    scalar: bool,
    /// `#[abi(hidden_return)]` is present.
    hidden_return: bool,
    /// Portable aggregate return lowering.
    aggregate: bool,
    /// Explicit calling convention override.
    convention: Option<syn::LitStr>,
    /// Attributes forwarded to the caller and the implementation method.
    forwarded: Vec<Attribute>,
}

impl MethodOptions {
    /// Classify the result and reject conflicting return-lowering requests.
    fn return_kind(&self, output: &ReturnType, name: &Ident) -> Result<ReturnKind, syn::Error> {
        if self.aggregate && (self.scalar || self.hidden_return) {
            return Err(syn::Error::new(
                name.span(),
                "aggregate cannot be combined with scalar or hidden_return",
            ));
        }
        let kind = classify_return(
            output,
            name,
            self.scalar || self.aggregate,
            self.hidden_return,
        )?;
        if self.aggregate {
            if matches!(output, ReturnType::Default) {
                return Err(syn::Error::new(
                    name.span(),
                    "aggregate needs a return type",
                ));
            }
            Ok(ReturnKind::Aggregate)
        } else {
            Ok(kind)
        }
    }

    /// Read one return-lowering option or calling-convention override.
    fn parse_abi_option(&mut self, item: Meta) -> Result<(), syn::Error> {
        match item {
            Meta::Path(path) if path.is_ident("scalar") => self.scalar = true,
            Meta::Path(path) if path.is_ident("hidden_return") => self.hidden_return = true,
            Meta::Path(path) if path.is_ident("aggregate") => self.aggregate = true,
            Meta::NameValue(pair) if pair.path.is_ident("convention") => {
                let text = parse_convention(&pair.value)?;
                if self.convention.is_some() {
                    return Err(syn::Error::new(
                        text.span(),
                        "convention may be specified only once",
                    ));
                }
                self.convention = Some(text);
            }
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    "abi: use scalar, hidden_return, aggregate, or convention = \"...\"",
                ));
            }
        }
        Ok(())
    }

    /// Read the attributes of a method.
    fn parse(attrs: &[Attribute]) -> Result<Self, syn::Error> {
        let mut options = Self {
            slot: None,
            scalar: false,
            hidden_return: false,
            aggregate: false,
            convention: None,
            forwarded: Vec::new(),
        };
        for attr in attrs {
            if attr.path().is_ident("doc") {
                continue;
            }
            if attr.path().is_ident("slot") {
                let value: syn::LitInt = attr.parse_args()?;
                options.slot = Some(value.base10_parse::<usize>()?);
                continue;
            }
            if attr.path().is_ident("abi") {
                let items =
                    attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
                for item in items {
                    options.parse_abi_option(item)?;
                }
                continue;
            }
            if FORWARDED_METHOD_ATTRIBUTES
                .iter()
                .any(|name| attr.path().is_ident(name))
            {
                options.forwarded.push(attr.clone());
                continue;
            }
            if attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr") {
                return Err(syn::Error::new(
                    attr.span(),
                    "a method of an interface cannot be configured out: removing it would \
                     shift the slots of every later method. Declare one interface for each \
                     configuration instead.",
                ));
            }
            return Err(syn::Error::new(
                attr.span(),
                "a method of an interface takes `#[slot(N)]`, `#[abi(...)]`, \
                 `#[deprecated]`, `#[must_use]`, `#[allow(...)]`, `#[expect(...)]`, and \
                 documentation only",
            ));
        }
        Ok(options)
    }
}

/// The method attributes forwarded to both the caller and the implementation method.
const FORWARDED_METHOD_ATTRIBUTES: &[&str] = &["deprecated", "must_use", "allow", "expect"];

/// Read a calling-convention override.
fn parse_convention(value: &Expr) -> Result<syn::LitStr, syn::Error> {
    let Expr::Lit(ExprLit {
        lit: Lit::Str(text),
        ..
    }) = value
    else {
        return Err(syn::Error::new(
            value.span(),
            "convention requires a string literal",
        ));
    };
    if !CONVENTIONS.contains(&text.value().as_str()) {
        return Err(syn::Error::new(
            text.span(),
            "unsupported convention; use C, system, cdecl, stdcall, fastcall, thiscall, win64, sysv64, or aapcs",
        ));
    }
    Ok(text.clone())
}

/// Read the `abi` argument of a macro.
fn parse_abi(value: &Expr) -> Result<Abi, syn::Error> {
    let name = path_expr_name(value).ok_or_else(|| {
        syn::Error::new(
            value.span(),
            "abi: give `com`, `cpp`, `msvc`, `itanium`, or `c`",
        )
    })?;
    Abi::from_name(&name).ok_or_else(|| {
        syn::Error::new(
            value.span(),
            format!("abi: `{name}` is unknown. Give `com`, `cpp`, `msvc`, `itanium`, or `c`."),
        )
    })
}

/// Split the outer attributes of an interface declaration into those that go to the
/// interface type and the `#[cfg]` attributes that go to every generated item.
///
/// Documentation is handled separately. Attributes that would break the generated type
/// are rejected.
fn forwarded_trait_attributes(
    attrs: &[Attribute],
) -> Result<(Vec<Attribute>, Vec<Attribute>), syn::Error> {
    let mut forwarded = Vec::new();
    let mut cfgs = Vec::new();
    for attr in attrs {
        let path = attr.path();
        if path.is_ident("doc") {
            continue;
        }
        if path.is_ident("cfg") {
            cfgs.push(attr.clone());
            continue;
        }
        let message = if path.is_ident("derive") {
            "an interface type cannot derive traits. It is never constructed or copied, and \
             it already implements `Debug`, `PartialEq`, and `Eq`."
        } else if path.is_ident("repr") {
            "an interface type is always `#[repr(transparent)]`"
        } else {
            forwarded.push(attr.clone());
            continue;
        };
        return Err(syn::Error::new(attr.span(), message));
    }
    Ok((forwarded, cfgs))
}

/// Tell if an attribute deprecates its item, possibly under `cfg_attr`.
pub(crate) fn is_deprecation(attr: &Attribute) -> bool {
    let path = attr.path();
    path.is_ident("deprecated")
        || (path.is_ident("cfg_attr")
            && mentions_ident(quote::ToTokens::to_token_stream(&attr.meta), "deprecated"))
}

/// Tell if a token stream holds an identifier, at any depth.
fn mentions_ident(tokens: TokenStream, name: &str) -> bool {
    tokens.into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(ident) => ident == name,
        proc_macro2::TokenTree::Group(group) => mentions_ident(group.stream(), name),
        proc_macro2::TokenTree::Punct(_) | proc_macro2::TokenTree::Literal(_) => false,
    })
}

/// The reference-count policy that `#[implement(..., refcount = ...)]` writes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RefCount {
    /// `SingleRefCount`.
    Single,
    /// `DualRefCount`.
    Dual,
}

/// The arguments of `#[implement(...)]`.
pub(crate) struct ImplementArgs {
    /// The implemented interfaces. The first one is the primary interface.
    pub(crate) interfaces: Vec<Path>,
    /// The declaration is inside `cppvtable-com`.
    pub(crate) internal: bool,
    /// The COM reference-count policy to implement, if any.
    pub(crate) refcount: Option<RefCount>,
}

impl ImplementArgs {
    /// Read the arguments.
    pub(crate) fn parse(tokens: TokenStream) -> Result<Self, syn::Error> {
        let span = tokens.span();
        let items = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
        let mut interfaces = Vec::new();
        let mut internal = false;
        let mut refcount = None;
        for item in items {
            match item {
                Meta::Path(path) if path.is_ident("internal") => internal = true,
                Meta::Path(path) => interfaces.push(path),
                Meta::NameValue(pair) if pair.path.is_ident("refcount") => {
                    if refcount.is_some() {
                        return Err(syn::Error::new(
                            pair.span(),
                            "refcount may be specified only once",
                        ));
                    }
                    refcount = Some(match path_expr_name(&pair.value).as_deref() {
                        Some("single") => RefCount::Single,
                        Some("dual") => RefCount::Dual,
                        _ => {
                            return Err(syn::Error::new(
                                pair.value.span(),
                                "refcount: use `single` or `dual`. Write `unsafe impl \
                                 RefCounted` by hand for `ForwardRefCount` or custom hooks.",
                            ));
                        }
                    });
                }
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "unknown argument. Give interface paths, `refcount = single|dual`, \
                         or `internal`.",
                    ));
                }
            }
        }
        if interfaces.is_empty() {
            return Err(syn::Error::new(
                span,
                "give at least one interface: `#[implement(IFoo)]`",
            ));
        }
        Ok(Self {
            interfaces,
            internal,
            refcount,
        })
    }

    /// Give the runtime path used by `#[implement]`.
    pub(crate) fn krate(&self, runtime: Runtime) -> TokenStream {
        if self.internal {
            quote::quote! { crate }
        } else {
            match runtime {
                Runtime::Abi => quote::quote! { ::cppvtable_abi },
                Runtime::Native => quote::quote! { ::cppvtable },
                Runtime::Com => quote::quote! { ::cppvtable_com },
            }
        }
    }

    /// Give the ABI runtime path used by generated vtable declarations.
    pub(crate) fn abi_krate(runtime: Runtime) -> TokenStream {
        match runtime {
            Runtime::Abi => quote::quote! { ::cppvtable_abi },
            Runtime::Com => quote::quote! { ::cppvtable_com },
            Runtime::Native => quote::quote! { ::cppvtable },
        }
    }
}

/// The arguments of `#[vtable_fn(...)]`.
pub(crate) struct VtableFnArgs {
    /// The binary interface whose vtable fields the function must match.
    pub(crate) abi: Abi,
    /// The per-method convention override that the matching vtable field uses.
    pub(crate) convention: Option<syn::LitStr>,
}

impl VtableFnArgs {
    /// Read the arguments.
    pub(crate) fn parse(tokens: TokenStream) -> Result<Self, syn::Error> {
        let span = tokens.span();
        let items = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(tokens)?;
        let mut abi = None;
        let mut convention = None;
        for item in &items {
            match item {
                Meta::NameValue(pair) if pair.path.is_ident("abi") && abi.is_none() => {
                    abi = Some(parse_abi(&pair.value)?);
                }
                Meta::NameValue(pair)
                    if pair.path.is_ident("convention") && convention.is_none() =>
                {
                    convention = Some(parse_convention(&pair.value)?);
                }
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "give `abi = cpp|c|msvc|itanium|com` once, and optionally the \
                         `convention = \"...\"` override of the method once",
                    ));
                }
            }
        }
        let abi = abi.ok_or_else(|| {
            syn::Error::new(
                span,
                "give the binary interface: `abi = cpp`, `c`, `msvc`, `itanium`, or `com`",
            )
        })?;
        Ok(Self { abi, convention })
    }
}

/// Keep the documentation attributes only.
fn doc_attributes(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .cloned()
        .collect()
}

/// Read the arguments after `&self`. A pattern that is not a name becomes `argN`.
fn method_params(signature: &syn::Signature) -> Vec<Param> {
    signature
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(position, argument)| {
            let FnArg::Typed(typed) = argument else {
                return None;
            };
            let name = match typed.pat.as_ref() {
                Pat::Ident(ident) => ident.ident.clone(),
                _ => format_ident!("arg{}", position, span = typed.pat.span()),
            };
            Some(Param {
                name,
                ty: (*typed.ty).clone(),
            })
        })
        .collect()
}

/// Give the name of an expression that is a single identifier.
fn path_expr_name(value: &Expr) -> Option<String> {
    let Expr::Path(path) = value else {
        return None;
    };
    path.path.get_ident().map(ToString::to_string)
}

/// Make the name of a generated item from the name of an interface.
///
/// `IFoo` and `Vtbl` give `IFooVtbl`.
pub(crate) fn derived_name(name: &Ident, suffix: &str) -> Ident {
    format_ident!("{}{}", name, suffix, span = name.span())
}

/// Make the path of a generated item from the path of an interface.
///
/// `a::b::IFoo` and `Impl` give `a::b::IFooImpl`.
pub(crate) fn derived_path(path: &Path, suffix: &str) -> Path {
    let mut result = path.clone();
    if let Some(segment) = result.segments.last_mut() {
        segment.ident = derived_name(&segment.ident, suffix);
    }
    result
}

/// Make the name of a shim function.
///
/// The name is snake case, so the lint `non_snake_case` never fires for it.
pub(crate) fn shim_name(interface: &Ident, index: usize) -> Ident {
    format_ident!(
        "__cppvtable_{}_slot_{}",
        interface.to_string().to_lowercase(),
        index,
        span = Span::call_site()
    )
}

/// Make the name of a generated static.
///
/// The name is upper case, so the lint `non_upper_case_globals` never fires for it.
pub(crate) fn static_name(type_name: &Ident, suffix: &str) -> Ident {
    format_ident!(
        "CPPVTABLE_{}_{}",
        type_name.to_string().to_uppercase(),
        suffix,
        span = Span::call_site()
    )
}
