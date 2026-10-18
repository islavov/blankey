//! The relay client in the tray app: the approval dialog, and status changes to the window and the tray.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use super::{show_main, tray};
use crate::config::Config;
use crate::core::Blankey;
use crate::i18n;
use crate::keychain::{self, OsKeyring};
use crate::relay::{Approval, Approver, Relay};

struct DialogApprover(AppHandle);

impl Approver for DialogApprover {
    fn approve(&self, approval: &Approval) -> bool {
        let Approval { client_name, redirect_host, relay_host, .. } = approval;
        let lang = super::state(&self.0).app.language();
        let text = i18n::tf(
            &lang,
            "{client} ({host}) wants to connect to Blankey on this computer through {relay}.",
            &[("client", client_name), ("host", redirect_host), ("relay", relay_host)],
        );
        let allowed = self
            .0
            .dialog()
            .message(text)
            .title(i18n::t(&lang, "Connect a web agent?"))
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(i18n::t(&lang, "Allow"), i18n::t(&lang, "Deny")))
            .blocking_show();
        let reason = i18n::tf(&lang, "allow {client} to connect to Blankey", &[("client", client_name)]);
        allowed && (!keychain::biometric_available() || keychain::authenticate(&reason))
    }
}

/// Creates the relay client as managed state and forwards its status; `Relay::start` connects it.
pub fn init(handle: &AppHandle, app: &Arc<Blankey>) -> Arc<Relay> {
    let tokens = OsKeyring::relay_token(&app.config.data_dir, &Config::default_data_dir());
    let runtime = tauri::async_runtime::handle().inner().clone();
    let approver = Arc::new(DialogApprover(handle.clone()));
    let relay = Arc::new(Relay::new(app.clone(), Box::new(tokens), approver, runtime));
    handle.manage(relay.clone());
    app.set_language(&i18n::resolve(relay.language().as_deref()));
    let mut status = relay.subscribe();
    let status_handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        while status.changed().await.is_ok() {
            let current = status.borrow_and_update().clone();
            let _ = status_handle.emit("relay-status", current);
            tray::refresh(&status_handle);
        }
    });
    let mut language = relay.subscribe_language();
    let (handle, app) = (handle.clone(), app.clone());
    tauri::async_runtime::spawn(async move {
        while language.changed().await.is_ok() {
            let resolved = i18n::resolve(language.borrow_and_update().as_deref());
            if resolved != app.language() {
                app.set_language(&resolved);
                tray::refresh(&handle);
                let _ = handle.emit("language-changed", resolved);
            }
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
