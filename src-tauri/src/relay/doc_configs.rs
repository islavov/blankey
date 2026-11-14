//! The document config set, pulled from the relay (`relay/README.md`, "Document config sets"). The app asks with
//! the engine version it has and the version of the set it holds, nothing else; what comes back is kept by
//! `docsets`.

use super::oauth::{self, Conditional, HttpError};
use super::{device_url, Relay};
use crate::doctypes::ENGINE;

impl Relay {
    /// Asks the relay for the current set: on every connection, every half hour while connected, and from
    /// Settings. A newer set is kept, the one held is confirmed, and one the account has no right to is dropped.
    /// Signed out, or with a relay that has no sets, nothing is asked.
    pub async fn refresh_doc_configs(&self) -> Result<(), String> {
        if !self.signed_in() {
            return Ok(());
        }
        let device = self.device_id()?;
        let held = self.app.doc_set_version();
        let answer = self
            .authorized(move |agent, discovery, token| {
                let Some(base) = discovery.device_doc_configs.as_deref() else { return Ok(None) };
                let mut url = url::Url::parse(&device_url(Some(base), &[], &device)?)
                    .map_err(|e| HttpError::Transport(e.to_string()))?;
                url.query_pairs_mut().append_pair("engine", &ENGINE.to_string());
                oauth::get_conditional(agent, url.as_str(), token, held.as_deref()).map(Some)
            })
            .await
            .map_err(|failure| failure.message())?;
        match answer {
            None => Ok(()),
            Some(Conditional::Fresh(body)) => self.app.store_doc_set(&body),
            Some(Conditional::Same) => {
                self.app.confirm_doc_set();
                Ok(())
            }
            Some(Conditional::Nothing | Conditional::Refused) => {
                self.app.drop_doc_set();
                Ok(())
            }
        }
    }
}
