//! Native Clang relative32 RTTI and explicit relative callback resolution.

use core::ffi::c_void;
use cppvtable::rtti::{RttiClass, RttiError, RttiMetadata, RttiVariant, relative_function};
use cppvtable::{implement, interface};

unsafe extern "C" {
    fn cppvtable_relative_primary() -> *mut c_void;
    fn cppvtable_relative_side() -> *mut c_void;
    fn cppvtable_relative_type() -> *const c_void;
    fn cppvtable_relative_typeid(object: *mut c_void) -> bool;
}

#[test]
fn native_relative_fixture_has_distinct_secondary_interface() {
    // SAFETY: All functions address the same process-lifetime native fixture.
    unsafe {
        let primary = cppvtable_relative_primary();
        let side = cppvtable_relative_side();
        assert_ne!(primary, side);
        assert!(!cppvtable_relative_type().is_null());
        assert!(cppvtable_relative_typeid(side));
        let primary_info =
            RttiMetadata::from_interface_variant(RttiVariant::ItaniumRelative32, primary);
        let side_info = RttiMetadata::from_interface_variant(RttiVariant::ItaniumRelative32, side);
        assert_eq!(primary_info.variant(), RttiVariant::ItaniumRelative32);
        assert_eq!(primary_info.type_info(), cppvtable_relative_type());
        assert_eq!(side_info.type_info(), primary_info.type_info());
        assert_eq!(primary_info.offset_to_top(), 0);
        assert_eq!(side_info.complete_object(side), primary);
        assert!(
            primary_info
                .mangled_name()
                .to_str()
                .unwrap()
                .contains("RelativeObject")
        );
        assert!(!primary_info.supports_pointer_tables());
    }
}

#[test]
fn rust_resolves_relative_function_slots_against_the_address_point() {
    type Method = unsafe extern "C" fn(*mut c_void, i32) -> i32;
    // SAFETY: The fixture uses the unsigned relative32 Itanium ABI. Its declared
    // signatures match Method, and its static object/table remain live throughout.
    unsafe {
        let primary = cppvtable_relative_primary();
        let side = cppvtable_relative_side();
        let primary_table = primary.cast::<*const i32>().read();
        let side_table = side.cast::<*const i32>().read();
        let first: Method = core::mem::transmute(relative_function(primary_table, 0));
        let next: Method = core::mem::transmute(relative_function(primary_table, 1));
        let second: Method = core::mem::transmute(relative_function(side_table, 0));
        assert_eq!(first(primary, 4), 11);
        assert_eq!(next(primary, 4), 15);
        assert_eq!(second(side, 4), 19);
    }
}

#[interface(abi = itanium)]
unsafe trait IPrimary {
    fn first(&self, amount: i32) -> i32;
    fn next(&self, amount: i32) -> i32;
}

#[interface(abi = itanium)]
unsafe trait ISide {
    fn second(&self, amount: i32) -> i32;
}

#[implement(IPrimary, ISide)]
struct Implementation;

impl IPrimaryImpl for Implementation {
    fn first(&self, amount: i32) -> i32 {
        7 + amount
    }
    fn next(&self, amount: i32) -> i32 {
        7 + 2 * amount
    }
}

impl ISideImpl for Implementation {
    fn second(&self, amount: i32) -> i32 {
        7 + 3 * amount
    }
}

#[test]
fn relative_metadata_cannot_silently_prefix_absolute_callback_tables() {
    // SAFETY: Valid static native metadata; this checked representation mismatch
    // is explicitly permitted by the builder and must fail before installation.
    unsafe {
        let primary = RttiMetadata::from_interface_variant(
            RttiVariant::ItaniumRelative32,
            cppvtable_relative_primary(),
        );
        let side = RttiMetadata::from_interface_variant(
            RttiVariant::ItaniumRelative32,
            cppvtable_relative_side(),
        );
        assert!(matches!(
            RttiClass::<Implementation>::builder()
                .with::<IPrimary>(primary)
                .with::<ISide>(side)
                .build(),
            Err(RttiError::UnsupportedVariant)
        ));
    }
}
