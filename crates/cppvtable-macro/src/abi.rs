//! The calling conventions of the supported binary interfaces.
//!
//! | `abi` argument | x86 | all other targets |
//! | -------------- | --- | ----------------- |
//! | `com` | `extern "system"` (stdcall) | `extern "system"` |
//! | `cpp` | `extern "thiscall"` (`this` in ECX) | `extern "C"` |
//! | `c` | `extern "C"` | `extern "C"` |
//!
//! `extern "thiscall"` exists on x86 only. The macro therefore makes two versions of the
//! vtable structure and of the shims of a `cpp` interface, one for each target group.
//! `com` and `c` need one version only.

use proc_macro2::TokenStream;
use quote::quote;

/// The binary interface of an interface declaration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Abi {
    /// A COM interface. The root of the chain is `IUnknown`.
    Com,
    /// A C++ class with virtual methods, built by MSVC.
    Cpp,
    /// A C table of function pointers. The first argument is `this`.
    C,
}

/// One version of the generated code: an optional `cfg` and the calling convention.
pub(crate) struct AbiVariant {
    /// The `cfg` attribute of this version. It is empty when one version is enough.
    pub(crate) cfg: TokenStream,
    /// The name of the calling convention, for example `system`.
    pub(crate) convention: &'static str,
}

impl Abi {
    /// Read the `abi` argument.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "com" => Some(Self::Com),
            "cpp" => Some(Self::Cpp),
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
            }],
            Self::C => vec![AbiVariant {
                cfg: TokenStream::new(),
                convention: "C",
            }],
            Self::Cpp => vec![
                AbiVariant {
                    cfg: quote! { #[cfg(target_arch = "x86")] },
                    convention: "thiscall",
                },
                AbiVariant {
                    cfg: quote! { #[cfg(not(target_arch = "x86"))] },
                    convention: "C",
                },
            ],
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
        assert_eq!(Abi::from_name("stdcall"), None);

        assert_eq!(Abi::Com.variants().len(), 1);
        assert_eq!(Abi::C.variants().len(), 1);
        assert_eq!(Abi::Cpp.variants().len(), 2);
        assert_eq!(Abi::Com.variants()[0].convention, "system");
        assert_eq!(Abi::Cpp.variants()[0].convention, "thiscall");
        assert_eq!(Abi::Cpp.variants()[1].convention, "C");
        assert!(Abi::Com.is_com());
        assert!(!Abi::Cpp.is_com());
    }
}
