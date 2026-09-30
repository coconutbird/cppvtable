//! Compiles C and C++ interoperability fixtures with the target compilers.

fn main() {
    for source in [
        "src/lib.rs",
        "src/single.rs",
        "src/multi.rs",
        "src/returns.rs",
        "src/inheritance.rs",
        "src/c_table.rs",
        "src/c_table.c",
        "src/com.rs",
    ] {
        println!("cargo:rerun-if-changed={source}");
    }
    cc::Build::new()
        .file("src/c_table.c")
        .compile("cppvtable_c_fixture");
    cpp_build::Config::new().build("src/lib.rs");
}
