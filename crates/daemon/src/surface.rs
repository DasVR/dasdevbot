//! Source checks for the card window, secret entry, and text-only rendering.
//! The Tauri crate is not a workspace member, so these files are read as data.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    fn desktop() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop")
    }

    fn read(path: &str) -> String {
        fs::read_to_string(desktop().join(path)).unwrap_or_else(|err| panic!("{path}: {err}"))
    }

    #[test]
    fn sign_is_card_only_and_secrets_are_main_or_settings() {
        let card: serde_json::Value =
            serde_json::from_str(&read("src-tauri/capabilities/card-window.json")).unwrap();
        let main: serde_json::Value =
            serde_json::from_str(&read("src-tauri/capabilities/main-window.json")).unwrap();
        let settings: serde_json::Value =
            serde_json::from_str(&read("src-tauri/capabilities/settings-window.json")).unwrap();
        let voice: serde_json::Value =
            serde_json::from_str(&read("src-tauri/capabilities/voice-window.json")).unwrap();
        assert_eq!(card["windows"], serde_json::json!(["card"]));
        assert_eq!(main["windows"], serde_json::json!(["main"]));
        assert_eq!(settings["windows"], serde_json::json!(["settings"]));
        assert_eq!(voice["windows"], serde_json::json!(["voice"]));
        let card_perms = card["permissions"].to_string();
        assert!(card_perms.contains("allow-sign-decision"));
        assert!(card_perms.contains("allow-undo-decision"));
        assert!(!card_perms.contains("set_secret"));
        assert!(!card_perms.contains("allow-set-secret"));
        for (name, value) in [("main", &main), ("settings", &settings)] {
            let perms = value["permissions"].to_string();
            assert!(perms.contains("allow-set-secret"), "{name}");
            assert!(!perms.contains("sign_decision"), "{name}");
            assert!(!perms.contains("allow-sign-decision"), "{name}");
            assert!(!perms.contains("allow-undo-decision"), "{name}");
        }
        assert!(voice["permissions"].as_array().unwrap().is_empty());
        let sign = read("src-tauri/permissions/allow-sign-decision.toml");
        let undo = read("src-tauri/permissions/allow-undo-decision.toml");
        let secret = read("src-tauri/permissions/allow-set-secret.toml");
        assert!(sign.contains("sign_decision"));
        assert!(undo.contains("undo_decision"));
        assert!(secret.contains("set_secret"));
        let conf = read("src-tauri/tauri.conf.json");
        assert!(conf.contains(
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost http://127.0.0.1:8787 http://localhost:8787; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"
        ));
        let commands = read("src-tauri/src/lib.rs");
        assert!(commands.contains("window.label()"));
        assert!(commands.contains("generate_handler!"));
        assert!(!commands.contains("card_window()"));
        assert!(
            std::fs::read_to_string(desktop().join("src-tauri/Cargo.toml"))
                .unwrap()
                .contains("tauri")
        );
        assert!(
            std::fs::read_to_string(desktop().join("src-tauri/src/main.rs"))
                .unwrap()
                .contains("dasdevbot_desktop_lib::run")
        );
        for label in ["\"main\"", "\"card\"", "\"settings\"", "\"voice\""] {
            assert!(conf.contains(label), "{label}");
        }
    }

    #[test]
    fn untrusted_content_is_text_and_the_secret_draft_is_not_stored() {
        let src = desktop().join("src");
        for entry in walk(&src) {
            let text = fs::read_to_string(&entry).unwrap();
            let name = entry.display().to_string();
            if name.ends_with(".svelte") {
                assert!(!text.contains("{@html}"), "{name}");
                assert!(!text.contains("innerHTML"), "{name}");
            }
        }
        let entry = read("src/lib/SecretEntry.svelte");
        assert!(!entry.contains("localStorage"));
        assert!(!entry.contains("svelte/store"));
        assert!(!entry.contains("plugin-store"));
        assert!(entry.contains("draft = \"\""));
        let api = read("src/lib/api.ts");
        assert!(api.contains("sign_decision"));
        assert!(api.contains("undo_decision"));
        assert!(api.contains("set_secret"));
        assert!(!api.contains("/v1/approvals/"));
    }

    fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else if path
                .extension()
                .is_some_and(|ext| ext == "svelte" || ext == "ts" || ext == "js")
            {
                out.push(path);
            }
        }
        out
    }
}
