//! Target-side sizing for explicitly sized partial vtables.

use proc_macro2::TokenStream;
use quote::quote;

/// Count the unknown trailing entries, including a base vtable in the total extent.
///
/// The expression is evaluated by the target compiler: the proc-macro host's pointer
/// width must not influence the layout of a cross-compiled interface.
pub(crate) fn trailing_slots(
    total: usize,
    declared: usize,
    base_type: Option<TokenStream>,
) -> TokenStream {
    let base_size = base_type.map_or_else(
        || quote! { 0usize },
        |base| quote! { ::core::mem::size_of::<#base>() },
    );
    quote! {
        {
            const ENTRY_SIZE: usize = ::core::mem::size_of::<unsafe extern "C" fn()>();
            const BASE_SIZE: usize = #base_size;
            const TOTAL_SLOTS: usize = #total;
            const DECLARED_SLOTS: usize = #declared;
            assert!(BASE_SIZE % ENTRY_SIZE == 0, "the base vtable must contain whole function-pointer slots");
            const BASE_SLOTS: usize = BASE_SIZE / ENTRY_SIZE;
            assert!(
                TOTAL_SLOTS >= DECLARED_SLOTS && BASE_SLOTS <= TOTAL_SLOTS - DECLARED_SLOTS,
                "slots is smaller than the inherited and declared vtable entries"
            );
            TOTAL_SLOTS - DECLARED_SLOTS - BASE_SLOTS
        }
    }
}
