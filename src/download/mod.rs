pub mod models;

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use models::Artifact;

/// Fetch `art` to `dir` over HTTPS, verifying its pinned SHA-256 before keeping it.
/// A mismatch deletes the bad file and errors — never load an unverified model.
pub fn fetch(art: &Artifact, dir: &Path) -> Result<(), String> {
    if art.sha256.starts_with("FILL") {
        return Err(format!("{} hash not pinned — run the pin step", art.name));
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let dest = dir.join(art.name);
    if dest.exists() && verify(&dest, art.sha256).unwrap_or(false) {
        return Ok(()); // already present + valid
    }
    if !art.url.starts_with("https://") {
        return Err(format!("refusing non-HTTPS url: {}", art.url));
    }
    // No redirects: a redirect could downgrade HTTPS, so it must error instead.
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let resp = agent.get(art.url).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let got = hex(Sha256::digest(&bytes));
    if got != art.sha256 {
        return Err(format!("checksum mismatch for {}: got {got}, want {}", art.name, art.sha256));
    }
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
    if art.name.ends_with(".tar.bz2") {
        extract_tar_bz2(&dest, dir)?;
    }
    Ok(())
}

/// Verify an existing file's SHA-256 (used before loading a cached model).
pub fn verify(path: &Path, want: &str) -> Result<bool, String> {
    if want.starts_with("FILL") {
        return Err(format!("{} hash not pinned — run the pin step", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(hex(Sha256::digest(&bytes)) == want)
}

/// Extract a verified `.tar.bz2` model archive into `dir`.
fn extract_tar_bz2(archive: &Path, dir: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|e| e.to_string())?;
    tar::Archive::new(bzip2::read::BzDecoder::new(file))
        .unpack(dir)
        .map_err(|e| e.to_string())
}

fn hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_matches_known_hash() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.bin");
        std::fs::write(&f, b"hello").unwrap();
        // sha256("hello")
        let want = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify(&f, want).unwrap());
        assert!(!verify(&f, "deadbeef").unwrap());
    }
}
