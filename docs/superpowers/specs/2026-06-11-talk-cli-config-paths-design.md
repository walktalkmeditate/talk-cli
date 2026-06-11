---
title: talk-cli config + state in XDG dirs (match meditate-cli)
date: 2026-06-11
status: design
origin: user request — config shouldn't live in the ~/talk reflections folder
---

# talk config + state in XDG dirs

**Goal:** Stop scattering machine-state files inside the user's reflection folder.
Move `config.toml`, `state.json`, and `streak.toml` out of `~/talk/` into the XDG
homes (`~/.config/talk`, `~/.local/share/talk`), exactly mirroring meditate-cli —
**without** a migration, since v0.1.0 has no installed base to preserve.

**Architecture:** Add `config_dir()` and `data_dir()` to `src/paths.rs` (the same
env-then-XDG-home resolution meditate uses), repoint `config_path()` and `state_path`
and the streak file at them, and create those dirs `0700` on first write. Reflection
`.md` files and the `.raw/` sidecar stay in `base_dir` (`~/talk`). No migration code.

**Tech stack:** Rust, `directories` crate (already a dep), `std::os::unix` perms.

---

## Background

Today everything lands in `base_dir` (`~/talk`, relocatable via `TALK_BASE_DIR`):

- `config.toml` — `config_path() = base_dir(None).join("config.toml")` (main.rs:552)
- `.state.json` — `state_path(base) = base.join(".state.json")` (main.rs:716; dot-prefixed to hide it inside `~/talk`)
- `.streak.toml` — `STREAK_FILE = ".streak.toml"` joined onto the base dir (streak.rs:7)

So the user's reflection folder is littered with hidden machine-state dotfiles.
meditate-cli instead splits config into `~/.config/meditate` and state/streak/packs
into `~/.local/share/meditate` (honoring `$XDG_CONFIG_HOME` / `$XDG_DATA_HOME`), plus a
one-time `migrate_legacy_dirs()`. talk should adopt the same split. talk has **no
installed base** (v0.1.0 just published, no real users; the author's own dogfooding
streak resetting is acceptable), so the migration is dropped — the single riskiest
part of meditate's version is simply not needed.

## Design

### New path resolvers (`src/paths.rs`)

```rust
/// Where `config.toml` lives: `$XDG_CONFIG_HOME/talk`, else `~/.config/talk`.
pub fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config")
}

/// Where `state.json` and `streak.toml` live: `$XDG_DATA_HOME/talk`, else
/// `~/.local/share/talk`.
pub fn data_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share")
}

/// `$ENV/talk` when the env var is a valid absolute path (reusing `safe_env_dir`'s
/// absolute + no-`..` check), else `~/<home_subdir>/talk`.
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

Reuses the existing `safe_env_dir` (rejects empty / relative / `..` env values — which
matches the XDG spec's "ignore a relative `$XDG_*_HOME`"). `directories` is already a
dependency. Because both resolvers derive from `$HOME` when the XDG vars are unset,
setting `HOME` to a temp dir isolates config, data, *and* `~/talk` together — the test
seam.

> talk is Unix-only today (the whole binary uses `std::os::unix`). The `~/.config` /
> `~/.local/share` layout is the right Unix home; a Windows-native fallback is out of
> scope until the deferred Windows port (which is a larger cross-platform effort).

### Repointed write sites

| File | From | To |
|------|------|----|
| `config.toml` | `base_dir(None).join("config.toml")` | `config_dir().join("config.toml")` |
| state | `state_path(base) = base.join(".state.json")` | `state_path() = data_dir().join("state.json")` |
| streak | `base.join(".streak.toml")` | `data_dir().join("streak.toml")` |

- `state_path` **loses its `base: &Path` parameter** — state no longer follows
  `TALK_BASE_DIR`; it lives in `data_dir()`. All five callers (main.rs:106, 155, 417,
  734, 738) drop the argument.
- The **dot prefix is dropped** (`.state.json` → `state.json`, `.streak.toml` →
  `streak.toml`). The dot existed only to hide these inside `~/talk` and skip
  vault-sync; in a dedicated data dir it is noise, and it matches meditate's
  `state.toml` / `streak.toml` naming.
- `STREAK_FILE` becomes `"streak.toml"`; the streak module resolves its path from
  `data_dir()` rather than receiving a base dir.

### Directory creation & perms

Before the first write to each new dir, ensure it exists `0700` — reuse the existing
`ensure_base_dir(dir)` (already path-generic; the name is a slight misnomer now but
renaming is out of scope). Files keep their `0600` `write_private`. So:

- `config init` / any config write → `ensure_base_dir(&config_dir())?` first.
- state and streak writes → `ensure_base_dir(&data_dir())?` first.

### What stays

- **Reflections** — the `.md` files and the `.raw/` sidecar — stay in `base_dir`
  (`~/talk`, still `TALK_BASE_DIR`-overridable). `~/talk` becomes *only* the readable
  journal.
- **Models** — stay in the cache dir (`models_dir()`, unchanged).
- The **first-run privacy disclosure** is unchanged: "your words land only in `~/talk`"
  remains true (only machine-state moved; words/raw stay in `~/talk`).

## Surface / docs

- `talk config path` prints `config_dir().join("config.toml")`; `talk config init`
  writes there (creating `~/.config/talk` `0700`).
- README: the config section notes config lives in `~/.config/talk` (or
  `$XDG_CONFIG_HOME/talk`); `talk config path` prints it.

## Non-goals

- **No migration / legacy-dir handling.** No installed base to preserve.
- No move of the reflection `.md` files or `.raw/` out of `~/talk`.
- No Windows-native dir fallback (Unix-only until the Windows port).
- No models relocation.

## Testing

- `paths`: `config_dir()` honors `$XDG_CONFIG_HOME` (absolute) and falls back to
  `~/.config/talk`; `data_dir()` honors `$XDG_DATA_HOME` and falls back to
  `~/.local/share/talk`; a relative/`..`/empty env value is ignored (falls back).
- Behavior: with `HOME` set to a temp dir (XDG vars unset), `config init` writes
  `<home>/.config/talk/config.toml`, a reflection writes its `.md` under `<home>/talk`,
  and a kept entry writes state to `<home>/.local/share/talk/state.json` and updates
  `<home>/.local/share/talk/streak.toml` — and `~/talk` contains **no** dotfiles.
- The new dirs are created `0700`; the files `0600` (extend the existing perms tests).
- Update any existing test that asserted config/state/streak under `base_dir` (and the
  privacy/no-egress tests that set `HOME`/`TALK_BASE_DIR` — they keep working since
  `HOME`-temp now isolates all three dirs).

## Files

- `src/paths.rs` — add `config_dir`, `data_dir`, `xdg_dir`; tests.
- `src/main.rs` — `config_path()` → `config_dir()`; `state_path()` drops its `base`
  param and uses `data_dir()`; ensure the dirs before writing; update the 5 state
  callers.
- `src/streak.rs` — `STREAK_FILE = "streak.toml"`; resolve from `data_dir()`; ensure
  `data_dir()` before writing.
- `README.md` — config-location note.
