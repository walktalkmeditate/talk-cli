use std::path::{Path, PathBuf};
use talk_core::entry::{append, Entry, Mode};
use talk_core::frontmatter::Frontmatter;
use crate::paths::write_private;

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
    pub ephemeral: bool,
}

/// Returns the written path, or None when ephemeral (nothing persisted).
pub fn write_entry(req: &WriteRequest) -> std::io::Result<Option<PathBuf>> {
    if req.ephemeral {
        return Ok(None);
    }
    let path = target_path(req.base, &req.target, req.date);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let raw = if req.keep_raw { req.raw } else { None };
    let entry = Entry { date: req.date, time: req.time, raw, clean: req.clean };

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

    write_private(&path, &new_contents)?;
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
            keep_raw: true, ephemeral: true,
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
                date, time: "08:14", raw: Some("um"), clean, keep_raw: true, ephemeral: false,
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
            raw: Some("secret"), clean: "Clean.", keep_raw: false, ephemeral: false,
        };
        let p = write_entry(&req).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("<!-- raw"));
    }
}
