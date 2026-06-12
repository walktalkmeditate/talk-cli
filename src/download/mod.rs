pub mod models;

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use models::Artifact;

/// Redirect hops to tolerate (GitHub release assets 302 once to their CDN).
const MAX_REDIRECTS: usize = 5;

/// Cap on a fetched body. The hash gate only runs after the body is fully read,
/// so without a cap a malicious hop could feed an endless (or gzip-amplified)
/// body into memory first. Generous headroom over the largest planned artifact
/// (the ~491 MB formatter model).
const MAX_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;

/// Validate a redirect target: only an absolute `https://` URL is followed, so a
/// hop can never downgrade the transport to plaintext. Scheme matching is
/// case-insensitive per RFC 3986 §3.1 (`HTTPS://` is valid); `get` keeps a
/// multi-byte char at the boundary fail-closed instead of panicking.
fn redirect_target(location: Option<&str>) -> Result<String, String> {
    match location {
        Some(l) if l.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("https://")) => {
            Ok(l.to_string())
        }
        Some(l) => Err(format!("refusing redirect to non-HTTPS url: {l}")),
        None => Err("redirect response without a Location header".to_string()),
    }
}

/// Read at most `cap` bytes from `r`, reporting bytes-so-far to `progress` as they
/// arrive; a longer body errors before the bytes ever reach the hash gate (memory
/// defense, not integrity — that's the SHA-256). Reads in chunks so the caller can
/// render download progress, never accumulating past `cap`.
fn read_capped(
    mut r: impl Read,
    cap: u64,
    name: &str,
    progress: &mut dyn FnMut(u64),
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if bytes.len() as u64 + n as u64 > cap {
            return Err(format!("response for {name} exceeds the {cap}-byte cap"));
        }
        bytes.extend_from_slice(&buf[..n]);
        progress(bytes.len() as u64);
    }
    Ok(bytes)
}

/// Fetch `art` to `dir` over HTTPS, verifying its pinned SHA-256 before keeping it.
/// A mismatch errors with the fetched bytes still in memory — nothing unverified
/// ever touches disk, so there is no bad file to clean up. `progress(done, total)`
/// is called as bytes arrive (`total` from Content-Length, `None` if absent).
pub fn fetch(
    art: &Artifact,
    dir: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<(), String> {
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
    // GitHub release assets 302-redirect to a CDN, so redirects must be followed —
    // but manually, validating EVERY hop stays HTTPS (a transport downgrade is
    // refused; the pinned SHA-256 is the integrity guarantee either way).
    // `identity` asks the server not to compress, so the hash is computed over
    // wire bytes and a gzip bomb can't amplify; the read cap bounds either way.
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_read(std::time::Duration::from_secs(600))
        .timeout_write(std::time::Duration::from_secs(30))
        .build();
    let get = |u: &str| {
        agent.get(u).set("Accept-Encoding", "identity").call().map_err(|e| e.to_string())
    };
    let mut url = art.url.to_string();
    let mut resp = get(&url)?;
    let mut hops = 0;
    while (300..400).contains(&resp.status()) {
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Err(format!("too many redirects fetching {}", art.name));
        }
        url = redirect_target(resp.header("location"))?;
        resp = get(&url)?;
    }
    let total = resp.header("Content-Length").and_then(|s| s.parse::<u64>().ok());
    let bytes = read_capped(resp.into_reader(), MAX_DOWNLOAD_BYTES, art.name, &mut |done| {
        progress(done, total)
    })?;
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

/// Re-extract a VERIFIED `.tar.bz2` archive into `dir` — the heal path when the
/// archive hashes clean but an extracted file went missing. Called only by the
/// live session's models gate (listen builds).
#[cfg_attr(not(feature = "listen"), allow(dead_code))]
pub fn extract(archive: &Path, dir: &Path) -> Result<(), String> {
    extract_tar_bz2(archive, dir)
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
    fn redirect_target_requires_absolute_https() {
        assert_eq!(
            redirect_target(Some("https://cdn.example.com/x")).unwrap(),
            "https://cdn.example.com/x"
        );
        // scheme is case-insensitive per RFC 3986
        assert!(redirect_target(Some("HTTPS://cdn.example.com/x")).is_ok());
        assert!(redirect_target(Some("http://cdn.example.com/x")).is_err()); // downgrade
        assert!(redirect_target(Some("HTTP://cdn.example.com/x")).is_err());
        assert!(redirect_target(Some("/relative/path")).is_err());
        assert!(redirect_target(Some("héllo://x")).is_err()); // non-boundary slice stays fail-closed
        assert!(redirect_target(None).is_err());
    }

    #[test]
    fn read_capped_rejects_oversized_bodies() {
        use std::io::Cursor;
        assert_eq!(read_capped(Cursor::new(b"0123456789"), 10, "x", &mut |_| {}).unwrap(), b"0123456789");
        assert!(read_capped(Cursor::new(b"0123456789!"), 10, "x", &mut |_| {}).is_err());
    }

    #[test]
    fn read_capped_reports_progress_monotonically_to_the_full_length() {
        use std::io::Cursor;
        let body = vec![7u8; 200_000]; // larger than one 64 KiB chunk → multiple reports
        let mut seen = Vec::new();
        let out = read_capped(Cursor::new(&body), 1_000_000, "x", &mut |done| seen.push(done)).unwrap();
        assert_eq!(out.len(), body.len());
        assert!(!seen.is_empty() && seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
        assert_eq!(*seen.last().unwrap(), body.len() as u64);
    }

    #[test]
    fn read_capped_enforces_the_cap_across_multiple_chunks() {
        use std::io::Cursor;
        // The cap spans more than one 64 KiB read, so the over-cap error must fire on
        // a LATER chunk — proving the memory bound holds beyond the first read.
        let body = vec![0u8; 200_000];
        assert!(read_capped(Cursor::new(&body), 100_000, "x", &mut |_| {}).is_err());
    }

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
