fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        // tauri-build embeds its app manifest (only the Common-Controls v6
        // dependency) as a resource linked into bins only. The lib's unit-test
        // harness then binds comctl32 v5 and fails to load with
        // STATUS_ENTRYPOINT_NOT_FOUND on the v6-only TaskDialogIndirect, which
        // tao/muda import. Embed the same dependency through the linker for
        // every linked target instead (tauri-apps/tauri#13419), and turn the
        // resource manifest off so the app binary does not get two.
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
        tauri_build::try_build(
            tauri_build::Attributes::new()
                .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
        )
        .expect("failed to run tauri-build");
    } else {
        tauri_build::build();
    }
}
