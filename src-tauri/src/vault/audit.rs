//! The audit log, the usage numbers reported from it, and backups.

use std::collections::BTreeMap;

use super::error::Result;
use super::Vault;

/// Audit actions the app reports to the relay as monthly numbers, and the kind each is reported as.
const REPORTED_ACTIONS: [(&str, &str); 4] = [
    ("generate", "doc_generated"),
    ("save_template", "template_saved"),
    ("save_fill_set", "fill_set_saved"),
    ("create_profile", "profile_created"),
];
const BACKUPS: usize = 3;

impl Vault {
    pub fn audit(&self, actor: &str, action: &str, target: &str) -> Result<()> {
        self.repo.insert_audit(actor, action, target)
    }

    /// How often each reported action happened per month (`YYYY-MM`, UTC) since the first of `months`'s oldest one,
    /// every month and kind present with a zero when nothing happened. Plain metadata, so it works while locked.
    pub fn usage_months(&self, months: &[String]) -> Result<BTreeMap<String, BTreeMap<&'static str, u64>>> {
        let mut report: BTreeMap<String, BTreeMap<&'static str, u64>> = months
            .iter()
            .map(|month| (month.clone(), REPORTED_ACTIONS.iter().map(|(_, kind)| (*kind, 0)).collect()))
            .collect();
        let Some(oldest) = months.iter().min() else { return Ok(report) };
        let actions: Vec<&str> = REPORTED_ACTIONS.iter().map(|(action, _)| *action).collect();
        for (month, action, n) in self.repo.audit_counts(oldest, &actions)? {
            let Some(counts) = report.get_mut(&month) else { continue };
            if let Some((_, kind)) = REPORTED_ACTIONS.iter().find(|(a, _)| *a == action) {
                counts.insert(kind, n);
            }
        }
        Ok(report)
    }

    /// Rotates vault.db.1 .. vault.db.3 and writes a fresh copy to vault.db.1.
    pub fn backup(&self) -> Result<()> {
        let name = self.path.file_name().and_then(|n| n.to_str()).unwrap_or("vault.db").to_string();
        let numbered = |i: usize| self.path.with_file_name(format!("{name}.{i}"));
        for i in (2..=BACKUPS).rev() {
            if numbered(i - 1).exists() {
                let _ = std::fs::rename(numbered(i - 1), numbered(i));
            }
        }
        self.repo.backup_to(&numbered(1))
    }

    /// (action, target) of every audit entry, oldest first.
    #[cfg(test)]
    pub fn audit_log(&self) -> Vec<(String, String)> {
        self.repo.audit_log().expect("audit log")
    }
}
