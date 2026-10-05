fn main() {
    tauri_build::build();
    // Tauri's application manifest is embedded into the product executable. Test
    // executables using AppHandle also link TaskDialogIndirect and need the v6
    // common-controls activation context; the system default v5 has no such export.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("tests.manifest");
        println!("cargo:rerun-if-changed=tests.manifest");
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}", manifest.display());
    }
}
