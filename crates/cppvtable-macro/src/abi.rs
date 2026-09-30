//! The calling conventions of the supported binary interfaces.
//!
//! | `abi` argument | x86 | all other targets |
//! | -------------- | --- | ----------------- |
//! | `com` | `extern "system"` (stdcall) | `extern "system"` |
//! | `cpp` on MSVC targets | `extern "thiscall"` (`this` in ECX) | `extern "C"` |
//! | `cpp` on Windows GNU targets | `extern "thiscall"` | `extern "C"` |
//! | `cpp` on other Itanium targets | `extern "C"` | `extern "C"` |
//! | `c` | `extern "C"` | `extern "C"` |
//!
//! `cpp` selects the Microsoft ABI on MSVC targets and the Itanium ABI otherwise.
//! `msvc` and `itanium` select those interfaces explicitly and reject incompatible
//! targets. The Microsoft ABI uses `thiscall` on x86; Itanium uses the C convention.
//! Windows GNU is the exception: its x86 Itanium methods use `thiscall`, while
//! retaining Itanium aggregate return placement rather than the Microsoft rules.
//!
//! A per-method `cdecl`, `stdcall`, `fastcall`, or `thiscall` override follows the C
//! header semantics of MSVC, clang, and GCC: it selects that convention on x86 and
//! lowers to `extern "C"` on every other architecture. The generated code then has an
//! x86 and a non-x86 version of each target configuration that does not already fix the
//! architecture.

use proc_macro2::{Span, TokenStream};
use quote::quote;

/// The overrides that name an x86 convention and lower to `"C"` elsewhere.
const X86_ONLY_CONVENTIONS: &[&str] = &["cdecl", "stdcall", "fastcall", "thiscall"];

/// Tell if a convention exists only on x86 and lowers to `"C"` on other targets.
pub(crate) fn is_x86_only(convention: &str) -> bool {
    X86_ONLY_CONVENTIONS.contains(&convention)
}

/// The binary interface of an interface declaration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Abi {
    /// A COM interface. The root of the chain is `IUnknown`.
    Com,
    /// A C++ class with virtual methods, using the target's C++ ABI.
    Cpp,
    /// The Microsoft C++ ABI, including clang-cl.
    Msvc,
    /// The Itanium C++ ABI used by Clang on Unix-like targets.
    Itanium,
    /// A C table of function pointers. The first argument is `this`.
    C,
}

/// One version of the generated code: an optional `cfg` and the calling convention.
pub(crate) struct AbiVariant {
    /// The `cfg` predicate of this version. `None` when one version is enough.
    predicate: Option<TokenStream>,
    /// Whether the predicate implies x86 (`Some(true)`), excludes it (`Some(false)`), or
    /// admits both (`None`).
    x86: Option<bool>,
    /// The name of the calling convention, for example `system`.
    pub(crate) convention: &'static str,
    /// Itanium C++ places an indirect aggregate result before `this`.
    pub(crate) hidden_before_this: bool,
    /// MSVC C++ member functions return trivial aggregates indirectly.
    pub(crate) aggregate_hidden: bool,
}

impl AbiVariant {
    /// Give the `cfg` attribute of this version. It is empty when one version is enough.
    pub(crate) fn cfg(&self) -> TokenStream {
        self.predicate
            .as_ref()
            .map_or_else(TokenStream::new, |predicate| quote! { #[cfg(#predicate)] })
    }

    /// Give the convention of a method, with its optional per-method override.
    ///
    /// An x86-only override lowers to `"C"` in a version that excludes x86.
    pub(crate) fn convention_for(&self, requested: Option<&syn::LitStr>) -> syn::LitStr {
        match requested {
            Some(requested) if self.x86 == Some(false) && is_x86_only(&requested.value()) => {
                syn::LitStr::new("C", requested.span())
            }
            Some(requested) => requested.clone(),
            None => syn::LitStr::new(self.convention, Span::call_site()),
        }
    }

    /// Split a version that admits both x86 and other architectures in two.
    fn split_x86(self) -> Vec<Self> {
        if self.x86.is_some() {
            return vec![self];
        }
        let (x86, other) = match &self.predicate {
            Some(predicate) => (
                quote! { all(#predicate, target_arch = "x86") },
                quote! { all(#predicate, not(target_arch = "x86")) },
            ),
            None => (
                quote! { target_arch = "x86" },
                quote! { not(target_arch = "x86") },
            ),
        };
        vec![
            Self {
                predicate: Some(x86),
                x86: Some(true),
                ..self
            },
            Self {
                predicate: Some(other),
                x86: Some(false),
                ..self
            },
        ]
    }
}

impl Abi {
    /// Read the `abi` argument.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "com" => Some(Self::Com),
            "cpp" => Some(Self::Cpp),
            "msvc" => Some(Self::Msvc),
            "itanium" => Some(Self::Itanium),
            "c" => Some(Self::C),
            _ => None,
        }
    }

    /// Tell if the interface comes from `IUnknown`.
    pub(crate) fn is_com(self) -> bool {
        self == Self::Com
    }

    /// Give the `compile_error!` that rejects an explicit C++ ABI on the wrong target.
    pub(crate) fn target_guard(self) -> TokenStream {
        match self {
            Self::Msvc => quote! {
                #[cfg(not(target_env = "msvc"))]
                compile_error!("abi = msvc requires an MSVC target; use abi = cpp for the target default");
            },
            Self::Itanium => quote! {
                #[cfg(target_env = "msvc")]
                compile_error!("abi = itanium requires a non-MSVC target; use abi = cpp for the target default");
            },
            Self::Com | Self::Cpp | Self::C => TokenStream::new(),
        }
    }

    /// Give the versions of the generated code.
    ///
    /// `split_x86` separates x86 from the other architectures in every version, which an
    /// x86-only convention override needs.
    pub(crate) fn variants(self, split_x86: bool) -> Vec<AbiVariant> {
        let variants = self.base_variants();
        if split_x86 {
            variants
                .into_iter()
                .flat_map(AbiVariant::split_x86)
                .collect()
        } else {
            variants
        }
    }

    /// Give the versions of the generated code before any x86 split.
    fn base_variants(self) -> Vec<AbiVariant> {
        match self {
            Self::Com => vec![AbiVariant {
                predicate: None,
                x86: None,
                convention: "system",
                hidden_before_this: false,
                aggregate_hidden: true,
            }],
            Self::C => vec![AbiVariant {
                predicate: None,
                x86: None,
                convention: "C",
                hidden_before_this: false,
                aggregate_hidden: false,
            }],
            Self::Cpp | Self::Msvc | Self::Itanium => {
                let variants = vec![
                    AbiVariant {
                        predicate: Some(quote! { all(target_arch = "x86", target_env = "msvc") }),
                        x86: Some(true),
                        convention: "thiscall",
                        hidden_before_this: false,
                        aggregate_hidden: true,
                    },
                    AbiVariant {
                        predicate: Some(
                            quote! { all(not(target_arch = "x86"), target_env = "msvc") },
                        ),
                        x86: Some(false),
                        convention: "C",
                        hidden_before_this: false,
                        aggregate_hidden: true,
                    },
                    AbiVariant {
                        predicate: Some(quote! { all(
                            not(target_env = "msvc"),
                            not(all(target_arch = "x86", target_os = "windows", target_env = "gnu"))
                        ) }),
                        x86: None,
                        convention: "C",
                        hidden_before_this: true,
                        aggregate_hidden: false,
                    },
                    AbiVariant {
                        predicate: Some(
                            quote! { all(target_arch = "x86", target_os = "windows", target_env = "gnu") },
                        ),
                        x86: Some(true),
                        convention: "thiscall",
                        hidden_before_this: true,
                        aggregate_hidden: false,
                    },
                ];
                match self {
                    Self::Msvc => variants.into_iter().take(2).collect(),
                    Self::Itanium => variants.into_iter().skip(2).collect(),
                    _ => variants,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Abi;

    #[test]
    fn the_names_map_to_the_conventions() {
        assert_eq!(Abi::from_name("com"), Some(Abi::Com));
        assert_eq!(Abi::from_name("cpp"), Some(Abi::Cpp));
        assert_eq!(Abi::from_name("c"), Some(Abi::C));
        assert_eq!(Abi::from_name("msvc"), Some(Abi::Msvc));
        assert_eq!(Abi::from_name("itanium"), Some(Abi::Itanium));
        assert_eq!(Abi::from_name("stdcall"), None);

        let cpp = Abi::Cpp.variants(false);
        assert_eq!(Abi::Com.variants(false).len(), 1);
        assert_eq!(Abi::C.variants(false).len(), 1);
        assert_eq!(cpp.len(), 4);
        assert_eq!(Abi::Com.variants(false)[0].convention, "system");
        assert_eq!(cpp[0].convention, "thiscall");
        assert_eq!(cpp[1].convention, "C");
        assert_eq!(cpp[2].convention, "C");
        assert!(!cpp[0].hidden_before_this);
        assert!(!cpp[1].hidden_before_this);
        assert!(cpp[2].hidden_before_this);
        assert_eq!(cpp[3].convention, "thiscall");
        assert!(cpp[3].hidden_before_this);
        assert!(!cpp[3].aggregate_hidden);
        assert!(Abi::Com.is_com());
        assert!(!Abi::Cpp.is_com());
    }

    #[test]
    fn x86_only_overrides_lower_to_c_outside_x86() {
        let stdcall = syn::LitStr::new("stdcall", proc_macro2::Span::call_site());
        let win64 = syn::LitStr::new("win64", proc_macro2::Span::call_site());

        let c = Abi::C.variants(true);
        assert_eq!(c.len(), 2);
        assert!(c[0].cfg().to_string().contains("target_arch = \"x86\""));
        assert!(
            c[1].cfg()
                .to_string()
                .contains("not (target_arch = \"x86\")")
        );
        assert_eq!(c[0].convention_for(Some(&stdcall)).value(), "stdcall");
        assert_eq!(c[1].convention_for(Some(&stdcall)).value(), "C");
        assert_eq!(c[1].convention_for(Some(&win64)).value(), "win64");
        assert_eq!(c[1].convention_for(None).value(), "C");

        // Only the Itanium version admits both architectures, so only it splits.
        let cpp = Abi::Cpp.variants(true);
        assert_eq!(cpp.len(), 5);
        let lowered: Vec<String> = cpp
            .iter()
            .map(|variant| variant.convention_for(Some(&stdcall)).value())
            .collect();
        assert_eq!(lowered, ["stdcall", "C", "stdcall", "C", "stdcall"]);
        assert_eq!(
            Abi::Com.variants(true)[1].convention_for(None).value(),
            "system"
        );
    }
}
