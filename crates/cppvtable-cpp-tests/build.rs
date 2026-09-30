//! Compiles the C++ interoperability test code.

fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/single.rs");
    println!("cargo:rerun-if-changed=src/multi.rs");
    cpp_build::build("src/lib.rs");
}
