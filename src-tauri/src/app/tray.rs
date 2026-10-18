//! The menu bar / system tray icon and its menu, rebuilt whenever the state changes.

use std::sync::atomic::Ordering;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Wry};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;

use super::{icons, online, platform, state};
use crate::i18n::{mark, t, tf};
use crate::keychain;
use crate::relay::State as RelayState;

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
    let lang = app.language();
    let item =
        |id: &str, text: &str, enabled: bool| MenuItem::with_id(handle, id, t(&lang, text), enabled, None::<&str>);
    let separator = || PredefinedMenuItem::separator(handle);

    let status = item("status", if unlocked { mark("Unlocked") } else { mark("Locked") }, false)?;
    let lock = if unlocked { item("lock", mark("Lock now"), true)? } else { item("unlock", mark("Unlock…"), true)? };
    let open_label = match pending {
        0 => t(&lang, "Open Blankey"),
        n => tf(&lang, "Open Blankey ({n} waiting)", &[("n", &n)]),
    };
    let open = MenuItem::with_id(handle, "open", open_label, true, None::<&str>)?;
    let url = app.config.mcp_url();
    let running = state.mcp_running.load(Ordering::SeqCst);
    let mcp_label = if running { tf(&lang, "MCP up: {url}", &[("url", &url)]) } else { t(&lang, "MCP down") };
    let mcp = MenuItem::with_id(handle, "mcp", mcp_label, false, None::<&str>)?;
    let connect = Submenu::with_items(
        handle,
        t(&lang, "Connect MCP"),
        true,
        &[
            &item("copy_address", mark("Copy MCP address"), true)?,
            &item("copy_claude_code", mark("Copy Claude Code connect command"), true)?,
            &item("copy_codex", mark("Copy Codex connect command"), true)?,
            &item("claude_desktop", mark("Configure Claude Desktop"), true)?,
        ],
    )?;
    let relay_menu = Submenu::new(handle, t(&lang, "Online"), true)?;
    if let Some(relay) = online::relay(handle) {
        let signed_in = relay.signed_in();
        let status = relay.status().state;
        let label = if signed_in { status.label() } else { RelayState::SignedOut.label() };
        relay_menu.append(&item("relay_status", label, false)?)?;
        if status == RelayState::AccountNeedsAttention {
            relay_menu.append(&item("relay_account", mark("Open account page…"), true)?)?;
        }
        relay_menu.append(&if signed_in {
            item("relay_logout", mark("Sign out"), true)?
        } else {
            item("relay_login", mark("Sign in to Blankey Online…"), true)?
        })?;
        relay_menu.append(&separator()?)?;
        relay_menu.append(&item("relay_agents", mark("Connected agents…"), true)?)?;
    }
    let keyring_label =
        if keychain::biometric_available() { mark("Unlock with Touch ID") } else { mark("Unlock with keychain") };
    let keyring_item =
        CheckMenuItem::with_id(handle, "keyring", t(&lang, keyring_label), unlocked, keyring, None::<&str>)?;
    let autostart_on = handle.autolaunch().is_enabled().unwrap_or(false);
    let autostart =
        CheckMenuItem::with_id(handle, "autostart", t(&lang, "Start at login"), true, autostart_on, None::<&str>)?;
    let quit = item("quit", mark("Quit"), true)?;
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
            &relay_menu,
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
    super::notify(handle, "Blankey", &t(&state(handle).app.language(), message));
}

fn on_menu(handle: &AppHandle, id: &str) {
    let state = state(handle);
    let url = state.app.config.mcp_url();
    let lang = state.app.language();
    match id {
        "open" => super::show_main(handle),
        "lock" => super::lock(handle),
        "unlock" => {
            super::show_main(handle);
            let handle = handle.clone();
            let reason = t(&lang, "unlock the Blankey vault");
            std::thread::spawn(move || {
                super::quick_unlock(&handle, &reason);
            });
        }
        "copy_address" => copy(handle, &url, mark("Address copied.")),
        "copy_claude_code" => copy(handle, &platform::claude_code_command(&url), mark("Command copied.")),
        "copy_codex" => copy(handle, &platform::codex_command(&url), mark("Command copied.")),
        "claude_desktop" => match platform::connect_claude_desktop() {
            Ok(path) => {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("claude_desktop_config.json");
                let text =
                    tf(&lang, "Blankey was added to {name}.\n\nRestart Claude Desktop to see it.", &[("name", &name)]);
                super::info(handle, &text);
            }
            Err(e) => super::alert(handle, &tf(&lang, "Could not write the config: {error}", &[("error", &e)])),
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
                super::alert(handle, &tf(&lang, "Could not change the login item: {error}", &[("error", &e)]));
            }
        }
        "relay_login" | "relay_logout" | "relay_account" => match (online::relay(handle), id) {
            (Some(relay), "relay_login") => relay.login(),
            (Some(relay), "relay_logout") => relay.logout(),
            (Some(relay), _) => {
                if let Some(url) = relay.cached_account_url() {
                    let _ = open::that(url);
                }
            }
            (None, _) => {}
        },
        "relay_agents" => online::show_connections(handle),
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
