//! The tray app: window lifecycle, MCP server, auto-lock and requests from AI assistants.

mod commands;
mod fill;
pub mod icons;
mod online;
mod platform;
mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;

use crate::config::{free_port, server_running, Config};
use crate::core::Blankey;
use crate::keychain::{self, OsKeyring};

const AUTO_LOCK_CHECK: Duration = Duration::from_secs(30);

pub struct AppState {
    pub app: Arc<Blankey>,
    pub mcp_running: AtomicBool,
    pub last_activity: Mutex<Instant>,
    /// a request dialog is open: no auto-lock meanwhile
    pub busy: AtomicBool,
}

impl AppState {
    pub fn touch(&self) {
        *self.last_activity.lock().unwrap() = Instant::now();
    }
}

pub fn state(handle: &AppHandle) -> tauri::State<'_, AppState> {
    handle.state::<AppState>()
}

/// Shows the main window and puts the app in the Dock while it is open.
pub fn show_main(handle: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = handle.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn hide_main(handle: &AppHandle) {
    if let Some(window) = handle.get_webview_window("main") {
        let _ = window.hide();
    }
    #[cfg(target_os = "macos")]
    let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

pub fn notify(handle: &AppHandle, title: &str, body: &str) {
    let _ = handle.notification().builder().title(title).body(body).show();
}

pub fn lock(handle: &AppHandle) {
    state(handle).app.vault().lock();
    platform::clear_opened_documents();
    let _ = handle.emit("vault-locked", ());
    tray::refresh(handle);
}

/// After an unlock: agents on the relay go to the computer unlocked last.
pub(super) fn claim_relay(handle: &AppHandle) {
    if let Some(relay) = online::relay(handle) {
        relay.claim_primary();
    }
}

/// Unlocks with the keychain-wrapped key; with a fingerprint reader it needs a Touch ID scan first.
pub fn quick_unlock(handle: &AppHandle, reason: &str) -> bool {
    let app = state(handle).app.clone();
    if !app.vault().keyring_enabled() {
        return false;
    }
    if keychain::biometric_available() && !keychain::authenticate(reason) {
        eprintln!("quick unlock: Touch ID was cancelled or failed");
        return false;
    }
    let unlocked = app.vault().unlock_with_keyring();
    if !unlocked {
        eprintln!("quick unlock: the keychain key is missing, unreadable or does not open the vault");
    }
    if unlocked {
        app.audit("user", "unlock_keychain", "");
        let _ = handle.emit("vault-unlocked", ());
        tray::refresh(handle);
        claim_relay(handle);
    }
    unlocked
}

fn on_request(handle: &AppHandle, id: i64) {
    let app = state(handle).app.clone();
    // the reason is sealed until the vault is unlocked
    let reason = app
        .vault()
        .get_request(id)
        .ok()
        .and_then(|r| r.payload.get("reason").and_then(|v| v.as_str()).map(str::to_string))
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "Your AI assistant sent a request. Open Blankey to review it.".into());
    notify(handle, "Request from your AI assistant", &reason);
    tray::refresh(handle);
    show_main(handle);
    let _ = handle.emit("request-created", id);
}

/// The MCP server, plus `extra` routes (the relay sign-in callback).
fn start_mcp(handle: &AppHandle, extra: axum::Router) {
    let state = state(handle);
    let app = state.app.clone();
    let port = app.config.port;
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => listener,
            Err(e) => {
                notify(&handle, "Blankey", &format!("The MCP server could not start on port {port}: {e}"));
                return;
            }
        };
        state_flag(&handle, true);
        let result = crate::mcp::serve_with(app, listener, extra, std::future::pending()).await;
        state_flag(&handle, false);
        if let Err(e) = result {
            notify(&handle, "Blankey", &format!("The MCP server stopped: {e}"));
        }
    });
}

fn state_flag(handle: &AppHandle, running: bool) {
    state(handle).mcp_running.store(running, Ordering::SeqCst);
    tray::refresh(handle);
}

fn start_auto_lock(handle: &AppHandle) {
    let handle = handle.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(AUTO_LOCK_CHECK);
        let state = state(&handle);
        let limit = Duration::from_secs(state.app.config.auto_lock_minutes as u64 * 60);
        let idle = state.last_activity.lock().unwrap().elapsed();
        let unlocked = state.app.vault().unlocked();
        if unlocked && idle > limit && !state.busy.load(Ordering::SeqCst) {
            lock(&handle);
        } else {
            tray::refresh(&handle);
        }
    });
}

pub fn quit(handle: &AppHandle) {
    state(handle).app.vault().lock();
    platform::clear_opened_documents();
    handle.exit(0);
}

pub fn run() {
    let mut config = Config::load().expect("cannot create the data directory");
    let already_running = server_running(&config.health_url());
    let mut moved_from = None;
    if !already_running {
        let port = free_port(config.port);
        if port != config.port {
            // saved, so the stdio bridge and the copied connect commands use the new address from now on
            moved_from = Some(config.port);
            config.port = port;
            let _ = config.save();
        }
    }
    let secrets = OsKeyring::for_data_dir(&config.data_dir, &Config::default_data_dir());
    let app = Arc::new(Blankey::new(config, Box::new(secrets)).expect("cannot open the vault"));
    {
        let mut vault = app.vault();
        // with Touch ID the vault stays locked until first use
        if vault.initialized() && !keychain::biometric_available() {
            vault.unlock_with_keyring();
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|handle, _args, _cwd| show_main(handle)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .manage(AppState {
            app: app.clone(),
            mcp_running: AtomicBool::new(false),
            last_activity: Mutex::new(Instant::now()),
            busy: AtomicBool::new(false),
        })
        .setup(move |tauri_app| {
            let handle = tauri_app.handle().clone();
            #[cfg(target_os = "macos")]
            tauri_app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            if already_running {
                let quit_handle = handle.clone();
                handle
                    .dialog()
                    .message("Blankey is already running. Look for its icon in the menu bar.")
                    .kind(MessageDialogKind::Info)
                    .title("Blankey")
                    .show(move |_| quit_handle.exit(0));
                return Ok(());
            }
            let relay = online::init(&handle, &app);
            tray::build(&handle)?;
            let hook_handle = handle.clone();
            app.set_request_hook(move |id| on_request(&hook_handle, id));
            let unlock_handle = handle.clone();
            app.set_unlock_hook(move |reason| {
                let _ = unlock_handle.emit("unlock-needed", reason);
                show_main(&unlock_handle);
            });
            start_mcp(&handle, relay.callback_router());
            relay.start();
            start_auto_lock(&handle);
            if let Some(old) = moved_from {
                let message = format!(
                    "Port {old} is in use, so Blankey moved to {}. If your AI assistant stops reaching Blankey, connect it again from the menu bar.",
                    app.config.mcp_url()
                );
                notify(&handle, "Blankey", &message);
            }
            let first_run = !app.vault().initialized();
            let pending = app.vault().count_requests("pending").unwrap_or(0) > 0;
            if first_run || pending {
                show_main(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // closing the window keeps the tray app running
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                hide_main(window.app_handle());
            }
        })
        .invoke_handler(commands::handlers())
        .build(tauri::generate_context!())
        .expect("error while building Blankey")
        .run(|handle, event| match event {
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => show_main(handle),
            RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
            _ => {}
        });
}

/// Shows an error box without blocking the caller.
pub fn alert(handle: &AppHandle, message: &str) {
    handle.dialog().message(message).kind(MessageDialogKind::Warning).title("Blankey").show(|_| {});
}

/// Shows an OK box without blocking the caller.
pub fn info(handle: &AppHandle, message: &str) {
    handle.dialog().message(message).kind(MessageDialogKind::Info).title("Blankey").show(|_| {});
}
