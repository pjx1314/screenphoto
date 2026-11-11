fn main() {
    println!(
        "cargo:rustc-env=SCREENPHOTO_TARGET={}",
        std::env::var("TARGET").expect("Cargo supplies TARGET")
    );
    println!("cargo:rerun-if-changed=build.rs");
}
