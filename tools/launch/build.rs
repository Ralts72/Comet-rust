fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("Cargo manifest directory");
    let build_script = std::path::Path::new(&manifest_dir).join("../tools/launch/build.rs");
    println!("cargo:rerun-if-changed={}", build_script.display());
    println!("cargo:rerun-if-env-changed=COMET_BUILD_ID");
    println!(
        "cargo:rustc-env=COMET_PROGRAM_BUILD_ID={}",
        std::env::var("COMET_BUILD_ID").unwrap_or_else(|_| "unprepared".into())
    );
    println!(
        "cargo:rustc-env=COMET_PROGRAM_BUILD_TARGET={}",
        std::env::var("TARGET").expect("Cargo target")
    );
}
