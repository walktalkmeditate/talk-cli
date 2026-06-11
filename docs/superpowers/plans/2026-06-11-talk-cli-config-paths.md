# talk Config + State in XDG Dirs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move `config.toml`, `state.json`, and `streak.toml` out of the `~/talk` reflection folder into the XDG homes (`~/.config/talk`, `~/.local/share/talk`), mirroring meditate-cli — with no migration (v0.1.0 has no installed base).

**Architecture:** Add `config_dir()` and `data_dir()` to `src/paths.rs` (env-then-XDG-home resolution via the existing `safe_env_dir`). Repoint `config_path()` at `config_dir()`, and `state_path()` + the streak file at `data_dir()`. Reflection `.md` files and `.raw/` stay in `base_dir` (`~/talk`). Tests set `HOME` and clear `XDG_*` so all dirs resolve under a tempdir.

**Tech Stack:** Rust, `directories` crate (already a dep), `std::os::unix` perms. Spec: `docs/superpowers/specs/2026-06-11-talk-cli-config-paths-design.md`.

**Per-commit invariant:** `cargo build`, `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` stay green after every task. Each task adds the resolver it uses (no dead-code warning) and updates the tests that read the moved file (no failing test).

---

## File Structure

- `src/paths.rs` — add `config_dir()` (Task 1) and `data_dir()` (Task 2), sharing an `xdg_dir()` helper; tests.
- `src/main.rs` — `config_path()` → `config_dir()` (Task 1); `state_path()` loses its `base` param → `data_dir()`, `disclose_once()` loses its `base` param, ensure `data_dir()` `0700`, streak callers → `data_dir()` (Task 2).
- `src/streak.rs` — `STREAK_FILE` de-dotted (Task 2).
- `tests/integration.rs` — `talk()` helper clears `XDG_*`; config writes move to `.config/talk` (Task 1).
- `tests/privacy.rs` — runner clears `XDG_*`; the state/streak existence check moves to `.local/share/talk` (Task 2).
- `README.md` — config-location note (Task 3).

---

## Task 1: `config_dir()` + repoint `config.toml`

**Files:** Modify `src/paths.rs`, `src/main.rs`, `tests/integration.rs`.

- [ ] **Step 1: Write the failing test** — add to the `tests` module in `src/paths.rs`:

```rust
    #[test]
    fn config_dir_honors_xdg_then_falls_back() {
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-config-test");
        assert_eq!(config_dir(), PathBuf::from("/tmp/xdg-config-test/talk"));
        std::env::set_var("XDG_CONFIG_HOME", "relative/dir"); // not absolute → ignored
        assert!(config_dir().ends_with(".config/talk"), "{:?}", config_dir());
        std::env::remove_var("XDG_CONFIG_HOME");
        assert!(config_dir().ends_with(".config/talk"), "{:?}", config_dir());
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p talk-cli --bin talk paths::tests::config_dir 2>&1 | head -20`
Expected: FAIL to compile — `config_dir` doesn't exist.

- [ ] **Step 3: Implement `config_dir` + `xdg_dir`** — add to `src/paths.rs`, after `models_dir`:

```rust
/// Where `config.toml` lives: `$XDG_CONFIG_HOME/talk`, else `~/.config/talk`.
pub fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config")
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
```

- [ ] **Step 4: Repoint `config_path()`** — in `src/main.rs`, replace the `config_path` function and its preceding comment (currently at lines 551-552):

```rust
// config.toml always lives at the default ~/talk; base_dir only relocates where entries land.
fn config_path() -> PathBuf { paths::base_dir(None).join("config.toml") }
```

with:

```rust
fn config_path() -> PathBuf { paths::config_dir().join("config.toml") }
```

(`handle_config`'s `ensure_base_dir(p.parent())` then creates `~/.config/talk` `0700` on `config init` — no change needed there.)

- [ ] **Step 5: Fix the config-reading integration tests** — in `tests/integration.rs`:

(a) Make the `talk()` helper clear the ambient XDG vars (so CI with a real `$XDG_CONFIG_HOME` doesn't escape the tempdir). Replace it with:

```rust
fn talk(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_talk"))
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .output().unwrap()
}
```

(b) Every test that writes a `config.toml` writes it under `<home>/talk/` today; move each to `<home>/.config/talk/`. Find them all: `grep -n 'config.toml' tests/integration.rs`. For each (e.g. lines 85-86, 110-111, 142, 161, 166, 179, 211, 339), change the directory from `talk` to `.config/talk` — both the `create_dir_all` and the `write` path. Example (lines 85-86):

```rust
    std::fs::create_dir_all(dir.path().join(".config/talk")).unwrap();
    std::fs::write(dir.path().join(".config/talk/config.toml"), "default_mode = \"journal\"\n").unwrap();
```

For tests that bind `let talk_dir = dir.path().join("talk");` and then `talk_dir.join("config.toml")`, write the config to `dir.path().join(".config/talk/config.toml")` instead (creating `dir.path().join(".config/talk")`), and leave the reflection-file assertions (`talk_dir.join("*.md")`) unchanged — reflections still land in `~/talk`.

- [ ] **Step 6: Verify green**

Run:
```
cargo test -p talk-cli --bin talk paths::tests::config_dir 2>&1 | tail -5
cargo test --workspace 2>&1 | grep -E "test result|error\[|FAILED" | tail -8
cargo clippy --all-targets -- -D warnings 2>&1 | tail -2
```
Expected: the new paths test passes; all integration tests pass (config now read from `~/.config/talk`); clippy clean.

- [ ] **Step 7: Commit**

```bash
git add src/paths.rs src/main.rs tests/integration.rs
git commit -m "feat(paths): config.toml moves to ~/.config/talk"
```

---

## Task 2: `data_dir()` + repoint state & streak

**Files:** Modify `src/paths.rs`, `src/main.rs`, `src/streak.rs`, `tests/privacy.rs`.

- [ ] **Step 1: Write the failing tests**

(a) In `src/paths.rs` `tests`:

```rust
    #[test]
    fn data_dir_honors_xdg_then_falls_back() {
        std::env::set_var("XDG_DATA_HOME", "/tmp/xdg-data-test");
        assert_eq!(data_dir(), PathBuf::from("/tmp/xdg-data-test/talk"));
        std::env::remove_var("XDG_DATA_HOME");
        assert!(data_dir().ends_with(".local/share/talk"), "{:?}", data_dir());
    }
```

(b) In `src/streak.rs` `tests`:

```rust
    #[test]
    fn streak_file_is_not_dot_prefixed() {
        assert_eq!(STREAK_FILE, "streak.toml");
        assert!(Streak::path_in(Path::new("/x")).ends_with("streak.toml"));
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p talk-cli data_dir_honors_xdg 2>&1 | head; cargo test -p talk-cli --bin talk streak::tests::streak_file 2>&1 | head -12`
Expected: `data_dir` doesn't exist (compile error); `STREAK_FILE` is still `.streak.toml` (assert fails).

- [ ] **Step 3: Add `data_dir()`** — in `src/paths.rs`, after `config_dir`:

```rust
/// Where `state.json` and `streak.toml` live: `$XDG_DATA_HOME/talk`, else
/// `~/.local/share/talk`.
pub fn data_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share")
}
```

- [ ] **Step 4: De-dot the streak file** — in `src/streak.rs` line 7:

```rust
pub const STREAK_FILE: &str = "streak.toml";
```

- [ ] **Step 5: Repoint state + streak in `src/main.rs`**

(a) `state_path` loses its `base` param and uses `data_dir()`. Replace (lines 716-717):

```rust
fn state_path(base: &Path) -> PathBuf {
    base.join(".state.json") // dot-prefixed so vault sync / indexing skip it
}
```

with:

```rust
fn state_path() -> PathBuf {
    paths::data_dir().join("state.json")
}
```

(b) `disclose_once` loses its `base` param. Replace its signature (line 733) `fn disclose_once(base: &Path) -> std::io::Result<()> {` with `fn disclose_once() -> std::io::Result<()> {`, and inside it change both `state_path(base)` calls to `state_path()`.

(c) Update every `state_path(base)` caller to `state_path()` (lines 106, 155, 417 — the disclose-internal ones at 734/738 were handled in (b)), and every `disclose_once(&base)` / `disclose_once(base)` caller to `disclose_once()` (lines 50, 78, 150, 238).

(d) Point the streak callers at `data_dir()`: change `streak::Streak::load_from(&base)` (line 61) to `streak::Streak::load_from(&paths::data_dir())`, and both `streak::record_entry(r.base, day)` (line 188) and `streak::record_entry(base, day)` (line 425) to `streak::record_entry(&paths::data_dir(), day)`.

(e) Ensure `data_dir()` exists `0700` before state/streak writes. At the session entry where the base dir is ensured (lines 31-32, `let base = paths::resolve_base(...)?; paths::ensure_base_dir(&base)?;`), add immediately after:

```rust
    paths::ensure_base_dir(&paths::data_dir())?;
```

(`record_entry` already `create_dir_all`s its dir, but only `ensure_base_dir` sets `0700`; `write_private` for state does not create the parent — so this line guarantees the data dir exists with the right perms before either writes.)

- [ ] **Step 6: Fix the privacy test** — in `tests/privacy.rs`:

(a) If its binary-runner helper sets `HOME` without clearing the XDG vars, add `.env_remove("XDG_CONFIG_HOME").env_remove("XDG_DATA_HOME")` to it (same reason as Task 1).

(b) The ephemeral test asserts state/streak were not written. Update the location + de-dotted names (lines 37-42):

```rust
    for name in ["state.json", "streak.toml"] {
        assert!(
            !dir.path().join(".local/share/talk").join(name).exists(),
            "{name} written by ephemeral"
        );
    }
```

(The `.raw/` check stays — `.raw/` is reflection data and remains in `~/talk`.)

- [ ] **Step 7: Verify green across feature sets**

Run:
```
cargo test --workspace 2>&1 | grep -E "test result|error\[|FAILED" | tail -8
cargo test --features listen --test privacy 2>&1 | tail -6
cargo clippy --all-targets -- -D warnings 2>&1 | tail -2
cargo clippy --features listen --all-targets -- -D warnings 2>&1 | tail -2
```
Expected: all pass; both clippy clean.

- [ ] **Step 8: Behavior smoke (manual)** — confirm the split end-to-end in a tempdir:

```
H=$(mktemp -d); env -u XDG_CONFIG_HOME -u XDG_DATA_HOME HOME="$H" cargo run --bin talk -- journal --from-text "x" --date 2026-06-11 --time 08:00 >/dev/null
ls "$H/talk" "$H/.local/share/talk" 2>&1
```
Expected: `$H/talk` holds only `2026-06-11.md` (no dotfiles); `$H/.local/share/talk` holds `state.json` + `streak.toml`.

- [ ] **Step 9: Commit**

```bash
git add src/paths.rs src/main.rs src/streak.rs tests/privacy.rs
git commit -m "feat(paths): state + streak move to ~/.local/share/talk"
```

---

## Task 3: README note + verification gate

**Files:** Modify `README.md`.

- [ ] **Step 1: Add a config-location note** — in `README.md`, in the section that mentions `talk config`, add a sentence: config lives at `~/.config/talk/config.toml` (or `$XDG_CONFIG_HOME/talk`); `talk config path` prints it. Keep it to one or two sentences consistent with the surrounding prose; do not restate the whole config flow.

- [ ] **Step 2: Full CI gate** (mirrors `.github/workflows/ci.yml`)

```
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo build --features listen && cargo test --features listen
```
Expected: all green.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: note config now lives in ~/.config/talk"
```

---

## Self-Review

**1. Spec coverage** — `config_dir`/`data_dir`/`xdg_dir` (Task 1/2); the three moves with de-dotted names (Task 1 config; Task 2 state+streak); `state_path` dropping its `base` param (Task 2 Step 5a/c); `0700` dir creation (Task 2 Step 5e — reusing `ensure_base_dir`); reflections + models staying put (untouched); `HOME`-temp test seam + clearing XDG (Task 1 Step 5a, Task 2 Step 6a); README note (Task 3); explicit non-goal "no migration" (no migration task exists). ✓

**2. Placeholder scan** — every code step shows complete code; the repetitive integration-test edits are specified by a `grep` + a concrete transformation rule + a worked example (the only non-verbatim step, because the ~9 sites differ in local-variable shape). No TBD/TODO. ✓

**3. Type consistency** — `config_dir()`/`data_dir()`/`xdg_dir()` defined in Task 1/2 are used verbatim after; `state_path()` is parameter-less everywhere from Task 2 on (signature + all 5 call sites + `disclose_once`'s two internal calls); `disclose_once()` is parameter-less at its definition and all 4 callers; `STREAK_FILE` is `"streak.toml"` and `Streak::path_in`/`record_entry`/`load_from` keep their dir-param API (the caller supplies `data_dir()`). ✓
