use std::io;
use std::path::{Path, PathBuf};

/// Resolve the base dir: explicit override, else `~/talk`.
pub fn base_dir(override_path: Option<PathBuf>) -> PathBuf {
    override_path.unwrap_or_else(|| {
        directories::UserDirs::new()
            .map(|d| d.home_dir().join("talk"))
            .unwrap_or_else(|| PathBuf::from("talk"))
    })
}

/// Resolve a config-supplied base dir, validating it. A configured path must be
/// absolute and under the user's home — rejecting `../..` traversal or an
/// arbitrary/cloud-synced location, per spec §13. `None` → the default `~/talk`.
pub fn resolve_base(configured: Option<&str>) -> io::Result<PathBuf> {
    let Some(raw) = configured else { return Ok(base_dir(None)) };
    let p = PathBuf::from(expand_tilde(raw));
    if !p.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "base_dir must be absolute"));
    }
    if let Some(dirs) = directories::UserDirs::new() {
        if !p.starts_with(dirs.home_dir()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "base_dir must be under your home directory",
            ));
        }
    }
    Ok(p)
}

fn expand_tilde(s: &str) -> String {
    match s.strip_prefix("~/") {
        Some(rest) => directories::UserDirs::new()
            .map(|d| d.home_dir().join(rest).to_string_lossy().into_owned())
            .unwrap_or_else(|| s.to_string()),
        None => s.to_string(),
    }
}

/// Create the base dir if missing, with 0700 perms set AT creation (no window
/// where it exists at the umask default).
pub fn ensure_base_dir(dir: &Path) -> io::Result<()> {
    if dir.exists() {
        return set_perms(dir, 0o700);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Write `contents` to `path`, creating the file with mode 0600 AT open time —
/// no world-readable TOCTOU window between create and a later chmod.
pub fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    let mut f = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true).create(true).truncate(true).mode(0o600).open(path)?
    };
    #[cfg(not(unix))]
    let mut f = std::fs::File::create(path)?;
    f.write_all(contents.as_bytes())
}

#[cfg(unix)]
fn set_perms(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_perms(_path: &Path, _mode: u32) -> io::Result<()> { Ok(()) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_dir_respects_override() {
        let p = base_dir(Some(PathBuf::from("/tmp/x")));
        assert_eq!(p, PathBuf::from("/tmp/x"));
    }

    #[test]
    fn resolve_base_rejects_relative_and_outside_home() {
        assert!(resolve_base(Some("../../etc")).is_err());      // not absolute
        assert!(resolve_base(Some("/etc")).is_err());           // outside home
        assert!(resolve_base(None).is_ok());                    // default ok
    }

    #[cfg(unix)]
    #[test]
    fn written_files_are_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        ensure_base_dir(dir.path()).unwrap();
        let f = dir.path().join("a.md");
        write_private(&f, "hi").unwrap();
        let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dmode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700);
    }
}
