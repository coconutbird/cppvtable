//! The attribute macros of the crate `cppvtable`.
//!
//! - `#[interface]` declares a binary interface: a COM interface, a C++ class with
//!   virtual methods, or a C table of function pointers.
//! - `#[implement]` gives an object the static vtables of one or more interfaces.
//!
//! Use the macros through `cppvtable`. That crate holds the documentation of the object
//! model and of the reference count policies.

mod abi;
mod implement;
mod interface;
mod parse;
mod validate;

use proc_macro::TokenStream;

/// Declare a binary interface.
///
/// ```ignore
/// #[interface(abi = com, iid = "d0223b96-bf7a-43fd-92bd-a43b0d82b9eb",
///             extends(IDirect3DResource9))]
/// pub unsafe trait IDirect3DVertexBuffer9 {
///     fn Lock(&self, offset: u32, size: u32, data: *mut *mut c_void, flags: u32) -> HRESULT;
///     fn Unlock(&self) -> HRESULT;
///     #[slot(6)]
///     fn GetDesc(&self, desc: *mut D3DVERTEXBUFFER_DESC) -> HRESULT;
/// }
/// ```
///
/// # Arguments
///
/// - `abi = com | cpp | c`: the calling convention. `com` uses `extern "system"`. `cpp`
///   uses `extern "thiscall"` on x86 and `extern "C"` on all other targets. `c` uses
///   `extern "C"`.
/// - `iid = "..."`: the interface identifier. A COM interface needs it. A `cpp` or `c`
///   interface may leave it out and then uses the zero GUID.
/// - `extends(IBase)`: the base interface. A COM interface without `extends` comes
///   directly from `IUnknown`.
/// - `root`: the interface has no base and the crate supplies the vtable builder. Only
///   `IUnknown` uses this.
/// - `internal`: the paths of the generated code start with `crate`. Only the crate
///   `cppvtable` uses this.
///
/// # Method attributes
///
/// - `#[slot(N)]`: put the method at the index `N` of the derived part of the vtable.
///   The macro fills the space with reserved entries.
/// - `#[abi(hidden_return)]`: the MSVC ABI gives the return value back through a hidden
///   pointer after `this`. Use it for a method that returns a structure.
/// - `#[abi(scalar)]`: the return type is a transparent wrapper of a number or of a
///   pointer, so the value goes back in a register.
///
/// # Generated items
///
/// See the module documentation of `cppvtable` for the list.
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

/// Give an object the static vtables of one or more interfaces.
///
/// ```ignore
/// #[implement(IDirect3DVertexBuffer9)]
/// pub struct VertexBuffer { /* Rust fields */ }
///
/// impl IDirect3DVertexBuffer9Impl for VertexBuffer { /* ... */ }
/// impl IDirect3DResource9Impl for VertexBuffer { /* each ancestor */ }
/// impl RefCounted for VertexBuffer { type Policy = DualRefCount; }
/// ```
///
/// The first interface of the list is the primary interface. Its vtable pointer is at
/// offset 0 of the object, so `QueryInterface` for `IUnknown` always gives that pointer.
/// This is the identity rule of COM.
///
/// Add `internal` to the list to make the paths of the generated code start with
/// `crate`. Only the crate `cppvtable` uses it.
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
