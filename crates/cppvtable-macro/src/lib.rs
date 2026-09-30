//! Shared generators for ABI declarations, ordinary C/C++ objects, and COM objects.
//!
//! `cppvtable-abi` reexports the caller-only macro. `cppvtable` reexports ordinary
//! interface/object generators. `cppvtable-com` reexports COM-specific generators.

mod abi;
mod implement;
mod interface;
mod layout;
mod parse;
mod validate;

use proc_macro::TokenStream;

/// Declare a binary interface.
///
/// ```ignore
/// #[interface(abi = com, iid = "d0223b96-bf7a-43fd-92bd-a43b0d82b9eb",
///             extends(IDirect3DResource9))]
/// pub unsafe trait IDirect3DVertexBuffer9 {
///     /// # Safety
///     /// `data` must be writable; offset, size, and flags must obey the resource contract.
///     unsafe fn Lock(&self, offset: u32, size: u32, data: *mut *mut c_void, flags: u32) -> HRESULT;
///     fn Unlock(&self) -> HRESULT;
///     /// # Safety
///     /// `desc` must point to writable storage for a descriptor.
///     #[slot(6)]
///     unsafe fn GetDesc(&self, desc: *mut D3DVERTEXBUFFER_DESC) -> HRESULT;
/// }
/// ```
///
/// # Arguments
///
/// - `abi = com`: the COM entry point uses `extern "system"`. The ordinary C/C++
///   entry points accept `cpp` (target default), `msvc`, `itanium`, or `c`.
///   MSVC x86 uses `extern "thiscall"`; other supported C/C++ targets use `extern "C"`.
///   Explicit MSVC/Itanium selections must agree with the Rust target environment.
/// - `iid = "..."`: required COM interface identifier; unavailable for ordinary objects.
/// - `extends(IBase)`: the base interface. A COM interface without `extends` comes
///   directly from `IUnknown`.
/// - `slots = N`: total function-pointer entries, including the base vtable. Unknown
///   trailing entries are reserved to reach this extent. Without it, the vtable ends
///   after its last declared entry. An insufficient extent is a compile error.
/// - `root`: the interface has no base. For COM, the runtime supplies the root vtable
///   builder; ordinary objects generate their own implementation shims.
/// - `internal`: the paths of the generated code start with `crate`. Only the
///   `cppvtable-com` runtime crate uses this.
///
/// # Method attributes
///
/// - `#[slot(N)]`: put the method at the index `N` of the derived part of the vtable.
///   The macro fills the space with reserved entries.
/// - `#[abi(hidden_return)]`: explicitly lower an indirect result pointer after `this`
///   for Microsoft C++/COM, or before `this` for Itanium C++. Use only when the foreign
///   signature matches that convention; prefer `aggregate` for trivial structures.
/// - `#[abi(scalar)]`: a transparent scalar wrapper returned using native scalar lowering.
/// - `#[abi(aggregate)]`: a trivially copyable `#[repr(C)]` structure returned according
///   to the selected C/C++ ABI. Nontrivial C++ classes require an explicit C shim.
/// - `#[abi(convention = "stdcall")]`: override this method's calling convention.
///   Accepted names are `C`, `system`, `cdecl`, `stdcall`, `fastcall`, `thiscall`,
///   `win64`, `sysv64`, and `aapcs`; the Rust target must support the selected convention.
///   This can be combined with a return-lowering option in the same attribute.
///
/// # Method safety
///
/// Generated implementation traits preserve each declaration's `fn` or `unsafe fn`.
/// Their methods may be called directly on standalone Rust values. A safe method must
/// support such calls and all arguments allowed by its signature. If a method requires
/// valid foreign pointers or an allocation-embedded `self`, declare it `unsafe fn` and
/// document those preconditions. Pointer parameters alone do not imply unsafety.
/// Foreign callers must uphold the method contract; every caller wrapper is unsafe.
///
/// # Generated items
///
/// See the module documentation of `cppvtable-com` for the full runtime contract.
#[proc_macro_attribute]
pub fn interface(args: TokenStream, item: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<syn::ItemTrait>(item) {
        Ok(parsed) => parsed,
        Err(error) => return error.to_compile_error().into(),
    };
    match interface::expand(args.into(), &parsed) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Declare an ABI interface without generating a Rust-owned object implementation.
///
/// Re-exported as `interface` by `cppvtable-abi`. The COM crate uses [`interface`]
/// instead, which also generates the implementation shims for `#[implement]`.
#[proc_macro_attribute]
pub fn interface_abi(args: TokenStream, item: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<syn::ItemTrait>(item) {
        Ok(parsed) => parsed,
        Err(error) => return error.to_compile_error().into(),
    };
    match interface::expand_abi(args.into(), &parsed) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Declare a callable and Rust-implementable C/C++ interface through `cppvtable`.
#[proc_macro_attribute]
pub fn interface_native(args: TokenStream, item: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<syn::ItemTrait>(item) {
        Ok(parsed) => parsed,
        Err(error) => return error.to_compile_error().into(),
    };
    match interface::expand_native(args.into(), &parsed) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Give an object the static vtables of one or more interfaces.
///
/// ```ignore
/// #[implement(IDirect3DVertexBuffer9)]
/// pub struct VertexBuffer { /* Rust fields */ }
///
/// impl IDirect3DVertexBuffer9Impl for VertexBuffer { /* ... */ }
/// impl IDirect3DResource9Impl for VertexBuffer { /* each ancestor */ }
/// unsafe impl RefCounted for VertexBuffer { type Policy = DualRefCount; }
/// ```
///
/// The first interface of the list is the primary interface. Its vtable pointer is at
/// offset 0 of the object, so `QueryInterface` for `IUnknown` always gives that pointer.
/// This is the identity rule of COM.
///
/// Add `internal` to the list to make the paths of the generated code start with
/// `crate`. Only the `cppvtable-com` runtime crate uses it.
#[proc_macro_attribute]
pub fn implement(args: TokenStream, item: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<syn::ItemStruct>(item) {
        Ok(parsed) => parsed,
        Err(error) => return error.to_compile_error().into(),
    };
    match implement::expand(args.into(), &parsed) {
        Ok(tokens) => tokens.into(),
        Err(error) => {
            let mut output = proc_macro2::TokenStream::new();
            quote::ToTokens::to_tokens(&parsed, &mut output);
            output.extend(error.to_compile_error());
            output.into()
        }
    }
}

/// Implement C/C++ interfaces through `cppvtable`, without a COM object model.
#[proc_macro_attribute]
pub fn implement_native(args: TokenStream, item: TokenStream) -> TokenStream {
    let parsed = match syn::parse::<syn::ItemStruct>(item) {
        Ok(parsed) => parsed,
        Err(error) => return error.to_compile_error().into(),
    };
    match implement::expand_native(args.into(), &parsed) {
        Ok(tokens) => tokens.into(),
        Err(error) => {
            let mut output = proc_macro2::TokenStream::new();
            quote::ToTokens::to_tokens(&parsed, &mut output);
            output.extend(error.to_compile_error());
            output.into()
        }
    }
}
