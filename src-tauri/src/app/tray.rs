//! The menu bar / system tray icon and its menu, rebuilt whenever the state changes.

use std::sync::atomic::Ordering;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;

use super::{icons, platform, state};
use crate::keychain;

const TRAY_ID: &str = "main";
const ICON_SIZE: u32 = 44;

fn icon(locked: bool, pending: bool) -> Image<'static> {
    let mono = cfg!(target_os = "macos");
    Image::new_owned(icons::draw(ICON_SIZE, locked, pending, mono), ICON_SIZE, ICON_SIZE)
}

fn menu(handle: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = state(handle);
    let app = &state.app;
    let (unlocked, pending, keyring) = {
        let vault = app.vault();
        (vault.unlocked(), vault.count_requests("pending").unwrap_or(0), vault.keyring_enabled())
    };
    let item = |id: &str, text: &str, enabled: bool| MenuItem::with_id(handle, id, text, enabled, None::<&str>);
    let separator = || PredefinedMenuItem::separator(handle);

    let status = item("status", if unlocked { "Unlocked" } else { "Locked" }, false)?;
    let lock = if unlocked { item("lock", "Lock now", true)? } else { item("unlock", "Unlock…", true)? };
    let open_label = if pending > 0 { format!("Open Blankey ({pending} from Claude)") } else { "Open Blankey".into() };
    let open = item("open", &open_label, true)?;
    let url = app.config.mcp_url();
    let running = state.mcp_running.load(Ordering::SeqCst);
    let mcp = item("mcp", &if running { format!("MCP up: {url}") } else { "MCP down".into() }, false)?;
    let connect = Submenu::with_items(
        handle,
        "Connect MCP",
        true,
        &[
            &item("copy_address", "Copy MCP address", true)?,
            &item("copy_claude_code", "Copy Claude Code connect command", true)?,
            &item("copy_codex", "Copy Codex connect command", true)?,
            &item("claude_desktop", "Configure Claude Desktop", true)?,
        ],
    )?;
    let keyring_label = if keychain::biometric_available() { "Unlock with Touch ID" } else { "Unlock with keychain" };
    let keyring_item = CheckMenuItem::with_id(handle, "keyring", keyring_label, unlocked, keyring, None::<&str>)?;
    let autostart_on = handle.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(handle, "autostart", "Start at login", true, autostart_on, None::<&str>)?;
    let quit = item("quit", "Quit", true)?;
    Menu::with_items(
        handle,
        &[
            &status,
            &lock,
            &separator()?,
            &open,
            &separator()?,
            &mcp,
            &connect,
            &separator()?,
            &keyring_item,
            &autostart,
            &separator()?,
            &quit,
        ],
    )
}

fn copy(handle: &AppHandle, text: &str, message: &str) {
    let _ = handle.clipboard().write_text(text);
    super::notify(handle, "Blankey", message);
}

fn on_menu(handle: &AppHandle, id: &str) {
    let state = state(handle);
    let url = state.app.config.mcp_url();
    match id {
        "open" => super::show_main(handle),
        "lock" => super::lock(handle),
        "unlock" => {
            super::show_main(handle);
            let handle = handle.clone();
            std::thread::spawn(move || {
                super::quick_unlock(&handle, "unlock the Blankey vault");
            });
        }
        "copy_address" => copy(handle, &url, "Address copied."),
        "copy_claude_code" => copy(handle, &platform::claude_code_command(&url), "Command copied."),
        "copy_codex" => copy(handle, &platform::codex_command(&url), "Command copied."),
        "claude_desktop" => match platform::connect_claude_desktop() {
            Ok(path) => {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("the config");
                super::notify(handle, "Blankey", &format!("Added to {name}. Restart Claude Desktop."));
            }
            Err(e) => super::alert(handle, &format!("Could not write the config: {e}")),
        },
        "keyring" => {
            let mut vault = state.app.vault();
            let result = if vault.keyring_enabled() { vault.disable_keyring() } else { vault.enable_keyring() };
            drop(vault);
            if let Err(e) = result {
                super::alert(handle, &e.to_string());
            }
        }
        "autostart" => {
            let launcher = handle.autolaunch();
            let result = if launcher.is_enabled().unwrap_or(false) { launcher.disable() } else { launcher.enable() };
            if let Err(e) = result {
                super::alert(handle, &format!("Could not change the login item: {e}"));
            }
        }
        "quit" => super::quit(handle),
        _ => {}
    }
    refresh(handle);
}

pub fn build(handle: &AppHandle) -> tauri::Result<()> {
    let locked = !state(handle).app.vault().unlocked();
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(locked, false))
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Blankey")
        .menu(&menu(handle)?)
        .show_menu_on_left_click(true)
        .on_menu_event(|handle, event| on_menu(handle, event.id().as_ref()))
        .build(handle)?;
    Ok(())
}

/// Rebuilds the menu and the icon from the current state.
pub fn refresh(handle: &AppHandle) {
    let Some(tray) = handle.tray_by_id(TRAY_ID) else { return };
    let state = state(handle);
    let (locked, pending) = {
        let vault = state.app.vault();
        (!vault.unlocked(), vault.count_requests("pending").unwrap_or(0) > 0)
    };
    let _ = tray.set_icon(Some(icon(locked, pending)));
    let _ = tray.set_icon_as_template(cfg!(target_os = "macos"));
    if let Ok(menu) = menu(handle) {
        let _ = tray.set_menu(Some(menu));
    }
}
