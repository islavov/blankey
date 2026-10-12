//! The relay client in the tray app: the approval dialog, and status changes to the window and the tray.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use super::{show_main, tray};
use crate::config::Config;
use crate::core::Blankey;
use crate::keychain::{self, OsKeyring};
use crate::relay::{Approval, Approver, Relay};

struct DialogApprover(AppHandle);

impl Approver for DialogApprover {
    fn approve(&self, approval: &Approval) -> bool {
        let Approval { client_name, redirect_host, relay_host, .. } = approval;
        let text = format!(
            "{client_name} ({redirect_host}) wants to connect to Blankey on this computer through {relay_host}."
        );
        let allowed = self
            .0
            .dialog()
            .message(text)
            .title("Connect a web agent?")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom("Allow".into(), "Deny".into()))
            .blocking_show();
        allowed
            && (!keychain::biometric_available()
                || keychain::authenticate(&format!("allow {client_name} to connect to Blankey")))
    }
}

/// Creates the relay client as managed state and forwards its status; `Relay::start` connects it.
pub fn init(handle: &AppHandle, app: &Arc<Blankey>) -> Arc<Relay> {
    let tokens = OsKeyring::relay_token(&app.config.data_dir, &Config::default_data_dir());
    let runtime = tauri::async_runtime::handle().inner().clone();
    let approver = Arc::new(DialogApprover(handle.clone()));
    let relay = Arc::new(Relay::new(app.clone(), Box::new(tokens), approver, runtime));
    handle.manage(relay.clone());
    let mut status = relay.subscribe();
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        while status.changed().await.is_ok() {
            let current = status.borrow_and_update().clone();
            let _ = handle.emit("relay-status", current);
            tray::refresh(&handle);
        }
    });
    relay
}

pub fn relay(handle: &AppHandle) -> Option<Arc<Relay>> {
    handle.try_state::<Arc<Relay>>().map(|relay| relay.inner().clone())
}

pub fn show_connections(handle: &AppHandle) {
    show_main(handle);
    let _ = handle.emit("show-page", "connections");
}
