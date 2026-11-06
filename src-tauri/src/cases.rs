//! Cases: one matter with the profiles in it, its files, templates, fill sets, documents and requests. What each view
//! of the vault sees (the global one, or one case), the operations that span the vault and the template folders, and
//! case files, which tools read through `casefile:<id>` handles decrypted into a private scratch folder.
//!
//! A case is not a security boundary: it sets what tools and pages show by default. Remote agents still read a file
//! only after the user allowed it for them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::json;

use crate::core::Blankey;
use crate::templates::Template;
use crate::vault::{CaseInfo, CollectedFile, Profile};

/// The media types of the case files tools can read, and their extension.
const READABLE: &[(&str, &str)] =
    &[("application/pdf", "pdf"), ("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "docx")];

/// pdf or docx for a file tools can read.
pub fn readable_kind(file_type: &str) -> Option<&'static str> {
    READABLE.iter().find(|(media, _)| *media == file_type).map(|(_, kind)| *kind)
}

/// The id in an exact `casefile:<id>`.
pub fn parse_case_file(source: &str) -> Option<i64> {
    let digits = source.strip_prefix("casefile:")?;
    let id: i64 = digits.parse().ok()?;
    (id > 0 && id.to_string() == digits).then_some(id)
}

pub fn case_file_source(id: i64) -> String {
    format!("casefile:{id}")
}

fn string(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The view a tool or page works in: global without a case, else one case.
pub struct Scope {
    pub case: Option<CaseInfo>,
    /// profile id -> role, for the profiles linked to the case
    roles: IndexMap<i64, String>,
    /// uuid -> id of every case, so a template of a deleted case counts as global
    uuids: HashMap<String, i64>,
}

impl Scope {
    pub fn case_id(&self) -> Option<i64> {
        self.case.as_ref().map(|c| c.id)
    }

    fn uuid(&self) -> Option<&str> {
        self.case.as_ref().map(|c| c.uuid.as_str())
    }

    /// The profile's role in this case; None when it is not linked.
    pub fn role(&self, profile_id: i64) -> Option<&str> {
        self.roles.get(&profile_id).map(String::as_str)
    }

    /// The profiles linked to the case, in the order they were added.
    pub fn linked(&self) -> impl Iterator<Item = i64> + '_ {
        self.roles.keys().copied()
    }

    /// Global profiles everywhere; in a case also the ones linked to it.
    pub fn sees_profile(&self, profile: &Profile) -> bool {
        profile.global || self.roles.contains_key(&profile.id)
    }

    /// The id of the case a template belongs to; None for a global one, or one whose case is gone.
    pub fn template_case(&self, template: &Template) -> Option<i64> {
        template.case.as_deref().and_then(|uuid| self.uuids.get(uuid)).copied()
    }

    /// Global templates everywhere; in a case also its own.
    pub fn sees_template(&self, template: &Template) -> bool {
        self.template_case(template).is_none_or(|id| Some(id) == self.case_id())
    }

    /// What this view may save over: in a case its own templates, without one the global ones.
    pub fn owns_template(&self, template: &Template) -> bool {
        self.template_case(template) == self.case_id()
    }

    /// Whether a row with this case id (a fill set, document, file or request) belongs to this view.
    pub fn holds(&self, case_id: Option<i64>) -> bool {
        case_id == self.case_id()
    }

    /// The case uuid a template made in this view gets.
    pub fn template_scope(&self) -> Option<&str> {
        self.uuid()
    }
}

/// What deleting a case removed.
#[derive(Serialize, Debug, PartialEq)]
pub struct DeletedCase {
    pub files: i64,
    pub fill_sets: i64,
    pub documents: i64,
    pub templates: usize,
    pub profiles_deleted: usize,
    pub profiles_unlinked: usize,
}

/// Writes `content` readable by this user only, in a folder only this user can open.
fn write_private(dir: &Path, path: &Path, content: &[u8]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(string)?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(string)?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(string)?;
        file.write_all(content).map_err(string)
    }
    #[cfg(not(unix))]
    std::fs::write(path, content).map_err(string)
}

impl Blankey {
    /// The global view without a case, else that case's.
    pub fn scope(&self, case_id: Option<i64>) -> Result<Scope, String> {
        let vault = self.vault();
        let cases = vault.list_cases().map_err(string)?;
        let case = match case_id {
            Some(id) => Some(
                cases
                    .iter()
                    .find(|c| c.id == id)
                    .cloned()
                    .ok_or(format!("Unknown case {id}; find_cases lists them"))?,
            ),
            None => None,
        };
        let roles = match case_id {
            Some(id) => vault
                .case_links()
                .map_err(string)?
                .into_iter()
                .filter(|l| l.case_id == id)
                .map(|l| (l.profile_id, l.role))
                .collect(),
            None => IndexMap::new(),
        };
        let uuids = cases.into_iter().map(|c| (c.uuid, c.id)).collect();
        Ok(Scope { case, roles, uuids })
    }

    pub fn visible_profiles(&self, scope: &Scope) -> Result<Vec<Profile>, String> {
        Ok(self.vault().list_profiles().map_err(string)?.into_iter().filter(|p| scope.sees_profile(p)).collect())
    }

    pub fn visible_templates(&self, scope: &Scope) -> Vec<Template> {
        self.templates.all().into_iter().filter(|t| scope.sees_template(t)).collect()
    }

    pub fn visible_template(&self, scope: &Scope, id: &str) -> Result<Template, String> {
        let template = self.templates.get(id)?;
        match scope.template_case(&template) {
            Some(case) if !scope.sees_template(&template) => {
                Err(format!("Template {id:?} belongs to case {case}; pass case_id {case}"))
            }
            _ => Ok(template),
        }
    }

    /// Every profile must be global or linked to the scope's case.
    pub fn check_visible(&self, scope: &Scope, profiles: impl IntoIterator<Item = i64>) -> Result<(), String> {
        let vault = self.vault();
        let links = vault.case_links().map_err(string)?;
        for id in profiles {
            let profile = vault.get_profile(id).map_err(string)?;
            if scope.sees_profile(&profile) {
                continue;
            }
            let cases: Vec<i64> = links.iter().filter(|l| l.profile_id == id).map(|l| l.case_id).collect();
            return Err(match (scope.case_id(), cases.as_slice()) {
                (None, [case]) => format!("Profile {id} belongs to case {case}; pass case_id {case}"),
                (None, _) => format!("Profile {id} belongs to cases {cases:?}; pass one of them as case_id"),
                (Some(_), _) => {
                    format!("Profile {id} belongs to another case; the user can add it to this one in Blankey")
                }
            });
        }
        Ok(())
    }

    /// A template id this view may write: a new one, or one it owns. Any other is refused with a free id.
    pub fn check_template_save(&self, scope: &Scope, id: &str) -> Result<(), String> {
        let Ok(existing) = self.templates.get(id) else { return Ok(()) };
        if scope.owns_template(&existing) {
            return Ok(());
        }
        let free = self.templates.free_id(id);
        Err(match scope.template_case(&existing) {
            None => format!(
                "{id:?} is a global template: save it without case_id to change it, or use the free id {free:?} for a \
                 template of this case"
            ),
            Some(_) => format!("The id {id:?} is taken by a template of another case; use the free id {free:?}"),
        })
    }

    // -- cases -----------------------------------------------------------------

    pub fn create_case(&self, title: &str) -> Result<i64, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("Give the case a title".into());
        }
        self.vault().create_case(title).map_err(string)
    }

    pub fn update_case(&self, id: i64, title: Option<&str>, notes: Option<&str>) -> Result<(), String> {
        let title = title.map(str::trim);
        if title == Some("") {
            return Err("Give the case a title".into());
        }
        self.vault().update_case(id, title, notes).map_err(string)
    }

    /// The case's profiles with their role and how many cases each is in, in the order they were added.
    pub fn case_profiles(&self, case_id: i64) -> Result<Vec<(Profile, String, usize)>, String> {
        let vault = self.vault();
        let links = vault.case_links().map_err(string)?;
        let profiles: HashMap<i64, Profile> =
            vault.list_profiles().map_err(string)?.into_iter().map(|p| (p.id, p)).collect();
        Ok(links
            .iter()
            .filter(|l| l.case_id == case_id)
            .filter_map(|l| {
                let cases = links.iter().filter(|other| other.profile_id == l.profile_id).count();
                Some((profiles.get(&l.profile_id)?.clone(), l.role.clone(), cases))
            })
            .collect())
    }

    /// The templates made in the case and not made global since.
    pub fn case_templates(&self, scope: &Scope) -> Vec<Template> {
        match scope.case_id() {
            Some(id) => self.templates.all().into_iter().filter(|t| scope.template_case(t) == Some(id)).collect(),
            None => Vec::new(),
        }
    }

    /// A profile local to the case, linked with its role.
    pub fn create_case_profile(&self, case_id: i64, name: &str, kind: &str, role: &str) -> Result<i64, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Enter a profile name".into());
        }
        self.vault().create_local_profile(name, kind.trim(), case_id, role.trim()).map_err(string)
    }

    pub fn link_profile(&self, case_id: i64, profile_id: i64, role: &str) -> Result<(), String> {
        self.vault().link_profile(case_id, profile_id, role.trim()).map_err(string)
    }

    /// Unlinks the profile; a local one that was on no other case is deleted with its fields and files. True when
    /// it was deleted.
    pub fn remove_case_profile(&self, case_id: i64, profile_id: i64) -> Result<bool, String> {
        let vault = self.vault();
        let profile = vault.get_profile(profile_id).map_err(string)?;
        vault.unlink_profile(case_id, profile_id).map_err(string)?;
        let elsewhere = vault.case_links().map_err(string)?.iter().any(|l| l.profile_id == profile_id);
        if profile.global || elsewhere {
            return Ok(false);
        }
        vault.delete_profile(profile_id).map_err(string)?;
        Ok(true)
    }

    pub fn make_template_global(&self, id: &str) -> Result<(), String> {
        self.templates.set_case(id, None).map(|_| ())
    }

    /// Deletes the case with what only it holds: its files, fill sets, documents and templates, and the local
    /// profiles linked to no other case. Global profiles and those of other cases are unlinked. Its pending requests
    /// are cancelled; done ones stay in the activity.
    pub fn delete_case(&self, id: i64) -> Result<DeletedCase, String> {
        let scope = self.scope(Some(id))?;
        let (only, unlinked) = {
            let vault = self.vault();
            let links = vault.case_links().map_err(string)?;
            let mut only = Vec::new();
            let mut unlinked = 0;
            for profile_id in scope.linked() {
                let global = vault.get_profile(profile_id).map_err(string)?.global;
                if !global && links.iter().filter(|l| l.profile_id == profile_id).count() == 1 {
                    only.push(profile_id);
                } else {
                    unlinked += 1;
                }
            }
            (only, unlinked)
        };
        let templates = self.case_templates(&scope);
        let counts = {
            let mut vault = self.vault();
            for request in vault.list_requests(Some("pending")).map_err(string)? {
                if request.case_id == Some(id) {
                    vault.resolve_request(request.id, "cancelled", json!({"outcome": "cancelled"})).map_err(string)?;
                }
            }
            let counts = vault.case_counts(id).map_err(string)?;
            vault.delete_case(id).map_err(string)?;
            for profile_id in &only {
                vault.delete_profile(*profile_id).map_err(string)?;
            }
            counts
        };
        // last: a template that fails to go shows as global
        for template in &templates {
            self.templates.delete(&template.id)?;
        }
        self.clear_scratch();
        let deleted = DeletedCase {
            files: counts.files,
            fill_sets: counts.fill_sets,
            documents: counts.documents,
            templates: templates.len(),
            profiles_deleted: only.len(),
            profiles_unlinked: unlinked,
        };
        self.audit(
            "user",
            "delete_case",
            &format!("case={id} {}", serde_json::to_string(&deleted).unwrap_or_default()),
        );
        Ok(deleted)
    }

    // -- case files ----------------------------------------------------------------

    /// Decrypted case files that tools read by path; cleared on start, on lock and when a case or file goes.
    fn scratch_dir(&self) -> PathBuf {
        self.config.data_dir.join("scratch")
    }

    pub fn clear_scratch(&self) {
        let _ = std::fs::remove_dir_all(self.scratch_dir());
    }

    /// A case file tools can read (pdf or docx), decrypted into the scratch folder; needs the vault unlocked.
    pub fn case_file(&self, source: &str) -> Result<(PathBuf, CollectedFile), String> {
        let unknown = || format!("Unknown case file {source:?}; get_case lists them");
        let id = parse_case_file(source).ok_or_else(unknown)?;
        let file = self.vault().get_collected_file(id).map_err(|_| unknown())?;
        if file.case_id.is_none() {
            return Err(unknown());
        }
        let kind = readable_kind(&file.file_type)
            .ok_or_else(|| format!("{source} is not a PDF or Word file, so it cannot be read"))?;
        let dir = self.scratch_dir();
        let path = dir.join(format!("casefile-{id}.{kind}"));
        if !path.is_file() {
            let (_, content) = self.vault().load_collected_file(id).map_err(string)?;
            write_private(&dir, &path, &content)?;
        }
        Ok((path, file))
    }

    /// Locks the vault and removes the decrypted case files.
    pub fn lock(&self) {
        self.vault().lock();
        self.clear_scratch();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::FILE;
    use crate::mcp::tools::tests::app;
    use crate::templates::{SaveTemplate, StrMap, TemplateKind};

    fn template(app: &Blankey, id: &str, case: Option<&str>) {
        app.templates
            .save(SaveTemplate {
                id,
                name: id,
                kind: TemplateKind::Typst,
                roles: StrMap::new(),
                fields: StrMap::new(),
                description: "",
                source_file: None,
                source_bytes: None,
                source_text: Some("x"),
                labels: StrMap::new(),
                boxes: vec![],
                origin: None,
                case,
            })
            .unwrap();
    }

    fn uuid(app: &Blankey, case: i64) -> String {
        app.vault().get_case(case).unwrap().uuid
    }

    #[test]
    fn scope_shows_global_or_linked() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let (a, b) = (app.create_case("Sale").unwrap(), app.create_case("Lease").unwrap());
        let global = app.vault().create_profile("Acme Ltd", "company").unwrap();
        let local = app.create_case_profile(a, "Jane Example", "person", "buyer").unwrap();
        let names = |case| -> Vec<String> {
            app.visible_profiles(&app.scope(case).unwrap()).unwrap().into_iter().map(|p| p.name).collect()
        };
        assert_eq!(names(None), ["Acme Ltd"]);
        assert_eq!(names(Some(a)), ["Acme Ltd", "Jane Example"]);
        assert_eq!(names(Some(b)), ["Acme Ltd"]);
        let scope = app.scope(Some(a)).unwrap();
        assert_eq!((scope.role(local), scope.role(global)), (Some("buyer"), None));
        assert!(app.check_visible(&app.scope(Some(b)).unwrap(), [local]).is_err());
        let error = app.check_visible(&app.scope(None).unwrap(), [local]).unwrap_err();
        assert!(error.contains(&format!("pass case_id {a}")), "{error}");
        assert!(app.scope(Some(999)).err().unwrap().contains("find_cases"));

        template(&app, "shared", None);
        template(&app, "deed", Some(&uuid(&app, a)));
        template(&app, "orphan", Some("a-case-that-is-gone"));
        let ids = |case| -> Vec<String> {
            app.visible_templates(&app.scope(case).unwrap()).into_iter().map(|t| t.id).collect()
        };
        assert_eq!(ids(None), ["orphan", "shared"]);
        assert_eq!(ids(Some(a)), ["deed", "orphan", "shared"]);
        assert_eq!(ids(Some(b)), ["orphan", "shared"]);
        let error = app.visible_template(&app.scope(None).unwrap(), "deed").unwrap_err();
        assert!(error.contains(&format!("pass case_id {a}")), "{error}");
    }

    #[test]
    fn template_ids_stay_in_their_scope() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let (a, b) = (app.create_case("Sale").unwrap(), app.create_case("Lease").unwrap());
        template(&app, "contract", None);
        template(&app, "deed", Some(&uuid(&app, a)));
        let (global, in_a, in_b) = (app.scope(None).unwrap(), app.scope(Some(a)).unwrap(), app.scope(Some(b)).unwrap());
        assert!(app.check_template_save(&global, "contract").is_ok());
        assert!(app.check_template_save(&in_a, "deed").is_ok());
        assert!(app.check_template_save(&in_b, "new-one").is_ok());
        let error = app.check_template_save(&in_a, "contract").unwrap_err();
        assert!(error.contains("without case_id") && error.contains("contract-2"), "{error}");
        let error = app.check_template_save(&in_b, "deed").unwrap_err();
        assert!(error.contains("another case") && error.contains("deed-2"), "{error}");
        assert!(app.check_template_save(&global, "deed").is_err());

        app.make_template_global("deed").unwrap();
        assert!(app.check_template_save(&global, "deed").is_ok());
    }

    #[test]
    fn delete_case_removes_what_only_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let (case, other) = (app.create_case("Sale").unwrap(), app.create_case("Lease").unwrap());
        let global = app.vault().create_profile("Acme Ltd", "company").unwrap();
        app.link_profile(case, global, "seller").unwrap();
        let only = app.create_case_profile(case, "Jane Example", "person", "buyer").unwrap();
        let shared = app.create_case_profile(case, "John Smith", "person", "witness").unwrap();
        app.link_profile(other, shared, "tenant").unwrap();
        template(&app, "deed", Some(&uuid(&app, case)));
        template(&app, "lease", Some(&uuid(&app, other)));
        let request = app.create_request(FILE, json!({"kind": "pdf", "reason": "r", "case_id": case}), None).unwrap();
        let scan = dir.path().join("deed.pdf");
        std::fs::write(&scan, b"%PDF").unwrap();
        app.import_case_files(case, &[(scan, "Seller, deed".into())]).unwrap();

        let deleted = app.delete_case(case).unwrap();
        let expected = DeletedCase {
            files: 1,
            fill_sets: 0,
            documents: 0,
            templates: 1,
            profiles_deleted: 1,
            profiles_unlinked: 2,
        };
        assert_eq!(deleted, expected);
        let vault = app.vault();
        assert!(vault.get_profile(only).is_err());
        assert!(vault.get_profile(shared).is_ok() && vault.get_profile(global).is_ok());
        assert_eq!(vault.get_request(request).unwrap().status, "cancelled");
        drop(vault);
        let ids: Vec<String> = app.templates.all().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["lease"]);
    }

    #[test]
    fn removing_a_local_profile_from_its_last_case_deletes_it() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let (a, b) = (app.create_case("Sale").unwrap(), app.create_case("Lease").unwrap());
        let global = app.vault().create_profile("Acme Ltd", "company").unwrap();
        let local = app.create_case_profile(a, "Jane Example", "person", "buyer").unwrap();
        app.link_profile(a, global, "seller").unwrap();
        app.link_profile(b, local, "tenant").unwrap();
        assert!(!app.remove_case_profile(a, global).unwrap());
        assert!(!app.remove_case_profile(a, local).unwrap(), "still in the lease");
        assert!(app.remove_case_profile(b, local).unwrap());
        assert!(app.vault().get_profile(local).is_err());
        assert!(app.vault().get_profile(global).is_ok());
    }

    #[test]
    fn case_files_import_checks_all_first_and_decrypt_to_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let case = app.create_case("Sale").unwrap();
        let (deed, photo, big) = (dir.path().join("deed.pdf"), dir.path().join("id.jpg"), dir.path().join("big.pdf"));
        std::fs::write(&deed, b"%PDF-deed").unwrap();
        std::fs::write(&photo, b"jpeg").unwrap();
        std::fs::File::create(&big).unwrap().set_len(crate::links::MAX_FILE_BYTES + 1).unwrap();
        let files = |paths: &[&PathBuf]| -> Vec<(PathBuf, String)> {
            paths.iter().map(|p| ((*p).clone(), "Seller, deed".to_string())).collect()
        };
        assert!(app.import_case_files(case, &files(&[&deed, &big])).unwrap_err().contains("big.pdf"));
        assert!(app.vault().list_collected_files().unwrap().is_empty(), "nothing is added when one file is too big");
        assert_eq!(app.import_case_files(case, &files(&[&deed, &photo])).unwrap(), 2);

        let stored = app.vault().list_collected_files().unwrap();
        let id = |name: &str| stored.iter().find(|f| f.name == name).unwrap().id;
        let (path, file) = app.case_file(&case_file_source(id("deed.pdf"))).unwrap();
        assert_eq!((file.case_id, file.description.as_str()), (Some(case), "Seller, deed"));
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-deed");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        assert!(app.case_file(&case_file_source(id("id.jpg"))).unwrap_err().contains("cannot be read"));
        assert!(app.case_file("casefile:999").is_err());

        app.lock();
        assert!(!path.exists(), "locking removes the decrypted copies");
    }

    #[test]
    fn request_file_in_a_case_imports_the_pick() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let case = app.create_case("Sale").unwrap();
        let payload = json!({"kind": "pdf", "reason": "The signed deed", "case_id": case});
        let request = app.create_request(FILE, payload, Some("g1")).unwrap();
        let pick = dir.path().join("Hiddenwell deed.pdf");
        std::fs::write(&pick, b"%PDF").unwrap();

        let result = app.store_upload(request, &pick, None).unwrap();
        let file_id = result["file_id"].as_str().unwrap();
        let file = app.vault().get_collected_file(parse_case_file(file_id).unwrap()).unwrap();
        assert_eq!((file.case_id, file.description.as_str()), (Some(case), "The signed deed"));
        assert!(!app.uploads_dir().exists(), "no plaintext copy in uploads");
        assert!(app.file_shared("g1", file_id), "the agent that asked may read it");
        assert!(!app.file_shared("g2", file_id));
    }

    #[test]
    fn case_file_handles_are_exact() {
        assert_eq!(parse_case_file("casefile:12"), Some(12));
        for bad in ["casefile:", "casefile:0", "casefile:012", "casefile:+3", "casefile:-3", "casefile:3/..", "file:3"]
        {
            assert_eq!(parse_case_file(bad), None, "{bad}");
        }
        assert_eq!(readable_kind("application/pdf"), Some("pdf"));
        assert_eq!(readable_kind("image/jpeg"), None);
    }
}
