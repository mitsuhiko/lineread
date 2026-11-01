// Compiles c/shim.c (variadic rl_message and libc macros Rust cannot call)
// into a static library that rustc bundles into liblineread.a.
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let cc = env::var("CC").unwrap_or_else(|_| "cc".into());
    let obj = out.join("shim.o");
    let lib = out.join("liblineread_shim.a");
    let st = Command::new(&cc)
        .args(["-c", "-O2", "-fPIC", "-Wall", "-o"])
        .arg(&obj)
        .arg("c/shim.c")
        .status()
        .expect("running the C compiler");
    assert!(st.success(), "compiling c/shim.c failed");
    let _ = std::fs::remove_file(&lib);
    let ar = env::var("AR").unwrap_or_else(|_| "ar".into());
    let st = Command::new(ar).arg("crs").arg(&lib).arg(&obj).status().expect("running ar");
    assert!(st.success(), "ar failed");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=lineread_shim");
    println!("cargo:rerun-if-changed=c/shim.c");
    println!("cargo:rerun-if-changed=build.rs");
}
