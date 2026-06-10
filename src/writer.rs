use std::path::{Path, PathBuf};
use talk_core::entry::{append, Entry, Mode};
use talk_core::frontmatter::Frontmatter;
use crate::paths::write_private;

#[derive(Clone, Copy)]
pub enum Target<'a> {
    /// Reflect: one file per question, bound by id.
    Reflect { id: &'a str, question: &'a str, slug: &'a str, pack: &'a str, addressee: &'a str },
    /// Journal: date-keyed.
    Journal,
}

pub struct WriteRequest<'a> {
    pub base: &'a Path,
    pub target: Target<'a>,
    pub date: &'a str,
    pub time: &'a str,
    pub raw: Option<&'a str>,
    pub clean: &'a str,
    pub keep_raw: bool,
    /// Route the verbatim raw to `~/talk/.raw/<filename>` instead of an inline
    /// comment in the main file (opt-in; `keep_raw=false` still means no raw).
    pub raw_sidecar: bool,
    pub ephemeral: bool,
}

/// Returns the written path, or None when ephemeral (nothing persisted).
pub fn write_entry(req: &WriteRequest) -> std::io::Result<Option<PathBuf>> {
    if req.ephemeral {
        return Ok(None);
    }
    let path = target_path(req.base, &req.target, req.date);
    // Defense-in-depth: the entry must land directly under base. A crafted slug
    // or date (e.g. "../x") would otherwise escape the base dir.
    if path.parent() != Some(req.base) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to write outside the base directory",
        ));
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let raw = if req.keep_raw { req.raw } else { None };
    let (inline_raw, sidecar_raw) = if req.raw_sidecar { (None, raw) } else { (raw, None) };
    let entry = Entry { date: req.date, time: req.time, raw: inline_raw, clean: req.clean };

    // Build (and thereby validate) the main file's new contents BEFORE touching
    // the sidecar: a corrupt main file must error before any sidecar append, so
    // deterministic failures leave no orphaned raw in `.raw/`.
    let new_contents = match &req.target {
        Target::Reflect { id, question, slug, pack, addressee } => {
            let (mut fm, body) = match Frontmatter::parse(&existing) {
                Some((fm, body)) => (fm, body.to_string()),
                None if existing.trim().is_empty() => (
                    Frontmatter {
                        id: id.to_string(), question: question.to_string(),
                        slug: slug.to_string(), pack: pack.to_string(),
                        addressee: addressee.to_string(), created: req.date.to_string(),
                        entries: 0, last: req.date.to_string(),
                    },
                    String::new(),
                ),
                None => return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} has unparseable frontmatter; refusing to overwrite", path.display()),
                )),
            };
            fm.entries += 1;
            fm.last = req.date.to_string();
            let new_body = append(&body, &entry, Mode::Reflect);
            format!("{}{}", fm.to_yaml(), new_body)
        }
        Target::Journal => append(&existing, &entry, Mode::Journal),
    };

    // Write the SIDECAR before the main file: a sidecar failure aborts before the
    // entry exists, so the raw words are never silently dropped from a main file
    // that nonetheless got written (the words stay in memory for the caller's
    // recovery path). `.raw/` is created 0700 at creation (no umask window).
    // If the main write then fails, the snapshot below rolls the sidecar back, so
    // a retry can't append the same raw twice.
    let sidecar_rollback = match sidecar_raw {
        Some(r) => {
            let raw_dir = req.base.join(".raw");
            crate::paths::ensure_base_dir(&raw_dir)?;
            let side = raw_dir.join(path.file_name().expect("entry paths have file names"));
            let prior = std::fs::read_to_string(&side).ok();
            let appended = format!("{}## {} {}\n{r}\n\n", prior.as_deref().unwrap_or_default(), req.date, req.time);
            write_private(&side, &appended)?;
            Some((side, prior))
        }
        None => None,
    };

    if let Err(e) = write_private(&path, &new_contents) {
        if let Some((side, prior)) = sidecar_rollback {
            let _ = match prior {
                Some(text) => write_private(&side, &text),
                None => std::fs::remove_file(&side),
            };
        }
        return Err(e);
    }
    Ok(Some(path))
}

fn target_path(base: &Path, target: &Target, date: &str) -> PathBuf {
    match target {
        Target::Reflect { slug, .. } => base.join(format!("{}.md", slug)),
        Target::Journal => base.join(format!("{}.md", date)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let req = WriteRequest {
            base: dir.path(),
            target: Target::Journal,
            date: "2026-06-08", time: "08:14",
            raw: Some("secret"), clean: "Secret.",
            keep_raw: true, raw_sidecar: false, ephemeral: true,
        };
        assert!(write_entry(&req).unwrap().is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn reflect_creates_then_appends_same_file() {
        let dir = tempfile::tempdir().unwrap();
        // A generic fn (not a closure) so the returned WriteRequest's borrows
        // tie to the inputs — closures can't express late-bound lifetimes.
        fn mk<'a>(base: &'a Path, date: &'a str, clean: &'a str) -> WriteRequest<'a> {
            WriteRequest {
                base,
                target: Target::Reflect {
                    id: "avoidance-core", question: "What am I avoiding?",
                    slug: "what-am-i-avoiding", pack: "examen", addressee: "self",
                },
                date, time: "08:14", raw: Some("um"), clean, keep_raw: true, raw_sidecar: false, ephemeral: false,
            }
        }
        write_entry(&mk(dir.path(), "2026-06-06", "First.")).unwrap();
        let p = write_entry(&mk(dir.path(), "2026-06-07", "Second.")).unwrap().unwrap();

        let text = std::fs::read_to_string(&p).unwrap();
        let (fm, _) = Frontmatter::parse(&text).unwrap();
        assert_eq!(fm.entries, 2);
        assert_eq!(fm.id, "avoidance-core");
        assert!(text.contains("## 2026-06-06") && text.contains("## 2026-06-07"));
    }

    #[test]
    fn keep_raw_false_omits_comment() {
        let dir = tempfile::tempdir().unwrap();
        let req = WriteRequest {
            base: dir.path(), target: Target::Journal,
            date: "2026-06-08", time: "08:14",
            raw: Some("secret"), clean: "Clean.", keep_raw: false, raw_sidecar: false, ephemeral: false,
        };
        let p = write_entry(&req).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("<!-- raw"));
    }

    fn sidecar_req<'a>(base: &'a Path, target: &'a Target<'a>) -> WriteRequest<'a> {
        WriteRequest {
            base, target: *target,
            date: "2026-06-09", time: "08:14",
            raw: Some("um the verbatim words"), clean: "The verbatim words.",
            keep_raw: true, raw_sidecar: true, ephemeral: false,
        }
    }

    #[test]
    fn corrupt_main_file_errs_before_any_sidecar_write() {
        let dir = tempfile::tempdir().unwrap();
        let target = Target::Reflect {
            id: "avoidance-core", question: "What am I avoiding?",
            slug: "what-am-i-avoiding", pack: "examen", addressee: "self",
        };
        std::fs::write(dir.path().join("what-am-i-avoiding.md"), "garbage, not frontmatter\n").unwrap();
        let err = write_entry(&sidecar_req(dir.path(), &target)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(!dir.path().join(".raw").exists(), ".raw/ must hold no residue after a refused write");
    }

    #[cfg(unix)]
    #[test]
    fn failed_main_write_rolls_back_the_sidecar_so_retry_appends_once() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = Target::Journal;
        // Pre-create `.raw/` writable, then lock the base dir: the sidecar append
        // succeeds but the main write can't create its tempfile.
        crate::paths::ensure_base_dir(&dir.path().join(".raw")).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        assert!(write_entry(&sidecar_req(dir.path(), &target)).is_err());
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!dir.path().join(".raw/2026-06-09.md").exists(), "rolled-back sidecar must be gone");

        write_entry(&sidecar_req(dir.path(), &target)).unwrap();
        let side = std::fs::read_to_string(dir.path().join(".raw/2026-06-09.md")).unwrap();
        assert_eq!(side.matches("## 2026-06-09").count(), 1, "retry must not duplicate the raw: {side}");
    }

    #[test]
    fn sidecar_routes_raw_out_of_the_main_file() {
        let dir = tempfile::tempdir().unwrap();
        let req = WriteRequest {
            base: dir.path(), target: Target::Journal,
            date: "2026-06-09", time: "08:14",
            raw: Some("um the verbatim words"), clean: "The verbatim words.",
            keep_raw: true, raw_sidecar: true, ephemeral: false,
        };
        let p = write_entry(&req).unwrap().unwrap();
        let main_text = std::fs::read_to_string(&p).unwrap();
        assert!(!main_text.contains("<!-- raw"));
        let side = dir.path().join(".raw").join(p.file_name().unwrap());
        let side_text = std::fs::read_to_string(&side).unwrap();
        assert!(side_text.contains("## 2026-06-09"));
        assert!(side_text.contains("um the verbatim words"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dmode = std::fs::metadata(dir.path().join(".raw")).unwrap().permissions().mode();
            assert_eq!(dmode & 0o777, 0o700);
        }
    }
}
