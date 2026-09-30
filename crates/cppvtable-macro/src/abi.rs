//! The calling conventions of the supported binary interfaces.
//!
//! | `abi` argument | x86 | all other targets |
//! | -------------- | --- | ----------------- |
//! | `com` | `extern "system"` (stdcall) | `extern "system"` |
//! | `cpp` on MSVC targets | `extern "thiscall"` (`this` in ECX) | `extern "C"` |
//! | `cpp` on Itanium targets | `extern "C"` | `extern "C"` |
//! | `c` | `extern "C"` | `extern "C"` |
//!
//! `cpp` selects the Microsoft ABI on MSVC targets and the Itanium ABI otherwise.
//! `msvc` and `itanium` select those interfaces explicitly and reject incompatible
//! targets. The Microsoft ABI uses `thiscall` on x86; Itanium uses the C convention.

use proc_macro2::TokenStream;
use quote::quote;

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
    /// The `cfg` attribute of this version. It is empty when one version is enough.
    pub(crate) cfg: TokenStream,
    /// The name of the calling convention, for example `system`.
    pub(crate) convention: &'static str,
    /// Itanium C++ places an indirect aggregate result before `this`.
    pub(crate) hidden_before_this: bool,
    /// MSVC C++ member functions return trivial aggregates indirectly.
    pub(crate) aggregate_hidden: bool,
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

    /// Give the versions of the generated code.
    pub(crate) fn variants(self) -> Vec<AbiVariant> {
        match self {
            Self::Com => vec![AbiVariant {
                cfg: TokenStream::new(),
                convention: "system",
                hidden_before_this: false,
                aggregate_hidden: true,
            }],
            Self::C => vec![AbiVariant {
                cfg: TokenStream::new(),
                convention: "C",
                hidden_before_this: false,
                aggregate_hidden: false,
            }],
            Self::Cpp | Self::Msvc | Self::Itanium => {
                let variants = vec![
                    AbiVariant {
                        cfg: quote! { #[cfg(all(target_arch = "x86", target_env = "msvc"))] },
                        convention: "thiscall",
                        hidden_before_this: false,
                        aggregate_hidden: true,
                    },
                    AbiVariant {
                        cfg: quote! { #[cfg(all(not(target_arch = "x86"), target_env = "msvc"))] },
                        convention: "C",
                        hidden_before_this: false,
                        aggregate_hidden: true,
                    },
                    AbiVariant {
                        cfg: quote! { #[cfg(not(target_env = "msvc"))] },
                        convention: "C",
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

        assert_eq!(Abi::Com.variants().len(), 1);
        assert_eq!(Abi::C.variants().len(), 1);
        assert_eq!(Abi::Cpp.variants().len(), 3);
        assert_eq!(Abi::Com.variants()[0].convention, "system");
        assert_eq!(Abi::Cpp.variants()[0].convention, "thiscall");
        assert_eq!(Abi::Cpp.variants()[1].convention, "C");
        assert_eq!(Abi::Cpp.variants()[2].convention, "C");
        assert!(!Abi::Cpp.variants()[0].hidden_before_this);
        assert!(!Abi::Cpp.variants()[1].hidden_before_this);
        assert!(Abi::Cpp.variants()[2].hidden_before_this);
        assert!(Abi::Com.is_com());
        assert!(!Abi::Cpp.is_com());
    }
}
