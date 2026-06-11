use std::io;
use std::path::{Path, PathBuf};

/// Resolve the base dir: explicit override, else `TALK_BASE_DIR`, else `~/talk`.
pub fn base_dir(override_path: Option<PathBuf>) -> PathBuf {
    if let Some(p) = override_path { return p; }
    if let Some(p) = safe_env_dir("TALK_BASE_DIR") { return p; }
    directories::UserDirs::new()
        .map(|d| d.home_dir().join("talk"))
        .unwrap_or_else(|| PathBuf::from("talk"))
}

/// Where downloaded models live (separate from journal entries). Honors a validated
/// `TALK_MODELS_DIR`, else the platform cache dir, else `<base>/models`.
#[cfg(any(feature = "listen", feature = "download"))]
pub fn models_dir() -> PathBuf {
    if let Some(p) = safe_env_dir("TALK_MODELS_DIR") { return p; }
    directories::ProjectDirs::from("org", "walktalkmeditate", "talk")
        .map(|d| d.cache_dir().join("models"))
        .unwrap_or_else(|| base_dir(None).join("models"))
}

/// Where `config.toml` lives: `$XDG_CONFIG_HOME/talk`, else `~/.config/talk`.
pub fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

/// Where `state.json` and `streak.toml` live: `$XDG_DATA_HOME/talk`, else
/// `~/.local/share/talk`.
pub fn data_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share")
}

/// `$<env_var>/talk` when the env var is a valid absolute path (reusing
/// `safe_env_dir`'s absolute + no-`..` check, which also ignores a relative
/// `$XDG_*_HOME` per the XDG spec), else `~/<home_subdir>/talk`.
fn xdg_dir(env_var: &str, home_subdir: &str) -> PathBuf {
    if let Some(base) = safe_env_dir(env_var) {
        return base.join("talk");
    }
    let mut dir = directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    for part in home_subdir.split('/') {
        dir.push(part);
    }
    dir.push("talk");
    dir
}

/// Read an env-supplied dir, accepting it only if absolute and free of `..`
/// components (reject traversal / relative paths). Warns + returns None otherwise.
fn safe_env_dir(var: &str) -> Option<PathBuf> {
    let raw = std::env::var(var).ok()?;
    if raw.is_empty() { return None; }
    let p = PathBuf::from(raw);
    if p.is_absolute() && !p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        Some(p)
    } else {
        eprintln!("{var} ignored: must be an absolute path without '..'");
        None
    }
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
    if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "base_dir must not contain '..'"));
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
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        #[cfg(unix)]
        let mut f = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true).create(true).truncate(true).mode(0o600).open(&tmp)?
        };
        #[cfg(not(unix))]
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
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
    fn base_dir_honors_and_validates_talk_base_dir() {
        std::env::set_var("TALK_BASE_DIR", "/tmp/talk-test-xyz");
        assert_eq!(base_dir(None), PathBuf::from("/tmp/talk-test-xyz"));
        std::env::set_var("TALK_BASE_DIR", "/tmp/../etc"); // traversal → rejected → not used
        assert_ne!(base_dir(None), PathBuf::from("/tmp/../etc"));
        std::env::set_var("TALK_BASE_DIR", "relative/dir");  // not absolute → rejected
        assert_ne!(base_dir(None), PathBuf::from("relative/dir"));
        std::env::remove_var("TALK_BASE_DIR");
        // explicit override always wins over the env
        std::env::set_var("TALK_BASE_DIR", "/tmp/ignored");
        assert_eq!(base_dir(Some(PathBuf::from("/tmp/explicit"))), PathBuf::from("/tmp/explicit"));
        std::env::remove_var("TALK_BASE_DIR");
    }

    #[test]
    fn resolve_base_rejects_relative_and_outside_home() {
        assert!(resolve_base(Some("../../etc")).is_err());      // not absolute
        assert!(resolve_base(Some("/etc")).is_err());           // outside home
        assert!(resolve_base(Some("/tmp/../etc")).is_err());    // '..' traversal
        assert!(resolve_base(None).is_ok());                    // default ok
    }

    #[test]
    fn config_dir_honors_xdg_then_falls_back() {
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-config-test");
        assert_eq!(config_dir(), PathBuf::from("/tmp/xdg-config-test/talk"));
        std::env::set_var("XDG_CONFIG_HOME", "relative/dir"); // not absolute → ignored
        assert!(config_dir().ends_with(".config/talk"), "{:?}", config_dir());
        std::env::remove_var("XDG_CONFIG_HOME");
        assert!(config_dir().ends_with(".config/talk"), "{:?}", config_dir());
    }

    #[test]
    fn data_dir_honors_xdg_then_falls_back() {
        std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-data-test");
        assert_eq!(data_dir(), PathBuf::from("/tmp/xdg-data-test/talk"));
        std::env::remove_var("XDG_DATA_HOME");
        assert!(data_dir().ends_with(".local/share/talk"), "{:?}", data_dir());
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
