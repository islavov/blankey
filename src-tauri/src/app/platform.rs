use std::path::PathBuf;

use serde_json::{json, Value};

pub fn claude_desktop_config_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
    #[cfg(target_os = "macos")]
    let base = dirs::home_dir().unwrap_or_default().join("Library/Application Support");
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let base = dirs::home_dir().unwrap_or_default().join(".config");
    base.join("Claude").join("claude_desktop_config.json")
}

/// Registers `blankey mcp` (stdio) as an MCP server in Claude Desktop's config, keeping other entries.
pub fn connect_claude_desktop() -> Result<PathBuf, String> {
    let path = claude_desktop_config_path();
    let mut config: Value = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(_) => json!({}),
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let servers = config
        .as_object_mut()
        .ok_or("The config is not a JSON object")?
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    let mut server = json!({"command": exe, "args": ["mcp"]});
    // the bridge and the app it starts must use this app's data dir
    if let Some(dir) = std::env::var_os("BLANKEY_DATA_DIR") {
        server["env"] = json!({"BLANKEY_DATA_DIR": dir.to_string_lossy()});
    }
    servers["blankey"] = server;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn claude_code_command(url: &str) -> String {
    format!("claude mcp add --transport http blankey {url}")
}

pub fn codex_command(url: &str) -> String {
    format!("codex mcp add blankey --url {url}")
}

/// Decrypted documents opened with the default app live here; cleared on lock and quit.
fn open_dir() -> PathBuf {
    std::env::temp_dir().join("blankey-open")
}

pub fn open_document(filename: &str, content: &[u8]) -> Result<(), String> {
    let dir = open_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let name = std::path::Path::new(filename).file_name().ok_or("Bad file name")?;
    let path = dir.join(name);
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    open::that_detached(&path).map_err(|e| e.to_string())
}

pub fn clear_opened_documents() {
    if let Ok(entries) = std::fs::read_dir(open_dir()) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The Dock icon of a debug build: `cargo run` starts a bare binary, which macOS shows as "exec". A bundled release
/// build already has the icon from its bundle.
#[cfg(all(target_os = "macos", debug_assertions))]
pub fn set_dock_icon() {
    use objc2::AllocAnyThread;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(main_thread) = MainThreadMarker::new() else { return };
    let data = NSData::with_bytes(include_bytes!("../../icons/icon.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else { return };
    unsafe { NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image)) };
}

pub fn reveal(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg("-R").arg(path).spawn().map(|_| ());
    #[cfg(not(target_os = "macos"))]
    let result = open::that_detached(if path.is_dir() { path } else { path.parent().unwrap_or(path) });
    result.map_err(|e| e.to_string())
}
