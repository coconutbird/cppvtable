//! Compiles the example's inline C++ code.

fn main() {
    // Build inline C++ code from the example binary.
    println!("cargo:rerun-if-changed=src/main.rs");
    cpp_build::build("src/main.rs");
}
