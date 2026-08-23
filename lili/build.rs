fn main() {
    configure_missing_frontend();
    let attributes =
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "sign_loopback_request",
            "begin_window_drag",
            "move_window_to",
            "commit_window_position",
            "open_pet_context_menu",
            "run_pet_context_action",
            "complete_desktop_smoke",
        ]));
    tauri_build::try_build(attributes).expect("failed to run Tauri build script");
}

fn configure_missing_frontend() {
    const FRONTEND_DIST: &str = "../dist";
    println!("cargo:rerun-if-changed={FRONTEND_DIST}");
    if std::path::Path::new(FRONTEND_DIST).is_dir() {
        return;
    }
    let mut config = match std::env::var("TAURI_CONFIG") {
        Ok(config) => serde_json::from_str(&config).expect("TAURI_CONFIG must contain valid JSON"),
        Err(std::env::VarError::NotPresent) => serde_json::json!({}),
        Err(std::env::VarError::NotUnicode(_)) => panic!("TAURI_CONFIG must contain valid UTF-8"),
    };
    let root = config
        .as_object_mut()
        .expect("TAURI_CONFIG must contain a JSON object");
    root.entry("build")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("TAURI_CONFIG build must contain a JSON object")
        .insert("frontendDist".to_owned(), serde_json::Value::Null);
    root.entry("bundle")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .expect("TAURI_CONFIG bundle must contain a JSON object")
        .insert("resources".to_owned(), serde_json::Value::Null);
    let config = serde_json::to_string(&config).expect("TAURI_CONFIG must serialize");
    println!("cargo:rustc-env=TAURI_CONFIG={config}");
    unsafe {
        std::env::set_var("TAURI_CONFIG", config);
    }
}
