//! Compiles C and C++ interoperability fixtures with the target compilers.

fn main() {
    for source in [
        "src/lib.rs",
        "src/single.rs",
        "src/multi.rs",
        "src/returns.rs",
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
    cpp_build::Config::new().build("src/lib.rs");
}
