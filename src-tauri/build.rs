fn main() {
    // Test executables need the Common Controls v6 manifest too: the resource
    // compiler only injects it into binaries, so a lib unit test importing
    // TaskDialogIndirect fails at load with STATUS_ENTRYPOINT_NOT_FOUND.
    let manifest =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    if std::env::var("CARGO_CFG_WINDOWS").is_ok()
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let attrs = tauri_build::WindowsAttributes::new_without_app_manifest();
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(attrs))
            .expect("failed to run tauri-build");
        for target in [
            "rustc-link-arg",
            "rustc-link-arg-bins",
            "rustc-link-arg-tests",
        ] {
            println!("cargo:{target}=/MANIFEST:EMBED");
            println!("cargo:{target}=/MANIFESTINPUT:{}", manifest.display());
        }
    } else {
        tauri_build::build();
    }
}
