//! Compiles C and C++ interoperability fixtures with the target compilers.

fn main() {
    for source in [
        "src/lib.rs",
        "src/single.rs",
        "src/multi.rs",
        "src/returns.rs",
        "src/rtti.rs",
        "src/rtti/smoke.rs",
        "src/rtti/cast.rs",
        "src/rtti/foreign.rs",
        "src/rtti/hook.rs",
        "src/rtti/relative.rs",
        "src/rtti/relative.cpp",
        "src/inheritance.rs",
        "src/c_table.rs",
        "src/conventions.rs",
        "src/c_table.c",
        "src/inline.c",
        "src/inline.rs",
        "src/com.rs",
    ] {
        println!("cargo:rerun-if-changed={source}");
    }
    cc::Build::new()
        .file("src/c_table.c")
        .file("src/inline.c")
        .compile("cppvtable_c_fixture");
    let mut native = cpp_build::Config::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Reference dynamic_cast failures are caught entirely inside the C++ fixture.
        native.flag("/EHsc");
    }
    native.build("src/lib.rs");
    println!("cargo:rustc-check-cfg=cfg(has_relative_vtables)");
    println!("cargo:rerun-if-env-changed=CPPVTABLE_REQUIRE_RELATIVE");
    let mut relative = cc::Build::new();
    relative.cpp(true);
    let has_relative = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && relative.get_compiler().is_like_clang()
        && relative
            .is_flag_supported("-fexperimental-relative-c++-abi-vtables")
            .unwrap_or(false);
    assert!(
        has_relative || std::env::var_os("CPPVTABLE_REQUIRE_RELATIVE").is_none(),
        "the requested relative-vtable tests need Linux Clang with relative C++ ABI support"
    );
    if has_relative {
        relative
            .file("src/rtti/relative.cpp")
            .flag("-fexperimental-relative-c++-abi-vtables")
            .compile("cppvtable_relative_fixture");
        println!("cargo:rustc-cfg=has_relative_vtables");
    }
}
