fn main() {
    // When a signed Firefox .xpi exists, set a cfg flag so the install code
    // can embed it via include_bytes!. The .xpi is produced by
    // `cargo xtask sign-extension` and is not checked into the repo.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let xpi = format!("{manifest_dir}/extension/attend.xpi");
    if std::path::Path::new(&xpi).exists() {
        println!("cargo:rustc-cfg=has_signed_xpi");
    }
    println!("cargo:rerun-if-changed=extension/attend.xpi");
}
