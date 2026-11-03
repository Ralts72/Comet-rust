fn main() {
    println!("cargo:rerun-if-env-changed=COMET_BUILD_ID");
    println!(
        "cargo:rustc-env=COMET_BUILD_ID={}",
        std::env::var("COMET_BUILD_ID").unwrap_or_else(|_| "unprepared".into())
    );
    println!(
        "cargo:rustc-env=COMET_BUILD_TARGET={}",
        std::env::var("TARGET").expect("Cargo target")
    );
}
