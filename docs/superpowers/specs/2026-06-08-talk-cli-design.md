# talk-cli — Design (v1, CLI)

> Spec date: 2026-06-08. Supersedes the open questions in
> [`DREAMING.md`](../../../DREAMING.md) with concrete decisions. The dreaming
> doc remains the source of *rationale*; this is the source of *plan*.

---

## 1. One-line

**talk = "the terminal that listens."** The third pillar (Walk · Talk ·
Meditate) in the terminal. You speak to yourself; it transcribes on-device; the
words settle into a quiet markdown file you can return to. Sibling to
`meditate-cli`.

`meditate-cli` gave the terminal a breath. `talk-cli` gives it an ear.

## 2. Scope of v1

- **Surface:** the **CLI** first. Web/PWA is a later, separate spec. (CLI-first
  over `DREAMING.md §9`'s web-first case because it reuses the proven
  `meditate-cli` workspace verbatim, validates `talk-core` against a real engine
  before the browser surface, and meets the developer audience where it already
  works — the web's better settle rendering is deferred, not dismissed.)
- **Identity:** the **talk pillar color is `rust`** — `rgb(160,99,75)` light /
  `rgb(196,126,99)` dark, taken directly from `pilgrim-ios`
  (`rust.colorset`), parallel to meditate's `moss`. `talk-core/palette.rs` is a
  clone of `meditate-core/palette.rs` with the base tone swapped `MOSS → RUST`,
  inheriting the same season/time-of-day tinting so the CLI rhymes with the iOS
  talk pillar and with the web later.
- **No orb.** The main screen is the live transcript itself. The words are the
  visual.
- **Local formatter included in v1** (constrained cleanup), with restraint as a
  first-class discipline.
- **Visible privacy** — zero-network during use is true by construction and
  shown in the UI.

## 3. Non-goals (explicit)

- System-wide dictation / "type into any app" (rejected in `DREAMING.md §1`).
- Any cloud or BYO-key formatter (off-ethos — never).
- Web/PWA surface (next spec; `talk-core` stays wasm-friendly).
- pilgrim-ios / pilgrim-android code sharing (later, via `talk-core`).
- Speaker diarization / multi-speaker.

## 4. Decisions locked in this session

| # | Decision | Choice |
|---|---|---|
| 1 | First surface | **CLI** (web later) |
| 2 | Main screen | **No orb** — live transcript + settle |
| 3 | Privacy | **Visible**, true by construction |
| 4 | Web | **PWA** (later phase) |
| 5 | Export format | **`.md` + YAML frontmatter** |
| 6 | Modes | **journal** (date-keyed) + **reflect** (question-keyed) |
| 7 | Reflect storage | **One file per question**, dated sections append chronologically |
| 8 | Landing | **Flat `~/talk/`**, configurable in `config.toml` |
| 9 | v1 formatter | **Included** (local LLM, Light default, raw always recoverable) |
| 10 | Engine stack | **cpal + sherpa-onnx (Moonshine + Silero VAD) + Candle** |
| 11 | v1 packs | **One flagship pack per direction** + 65-spine + **ephemeral mode**; rest via `talk download` |

## 5. Architecture

Mirrors `meditate-cli` bone-for-bone (proven workspace, same Homebrew tap + npm).

```
talk-cli/
  Cargo.toml                  workspace root = the `talk` CLI package
  src/                        the CLI binary (clap, terminal, I/O facades)
    main.rs  cli.rs  config.rs  session.rs  state.rs  streak.rs
    paths.rs  keymap.rs  term.rs
    audio/      cpal capture + (optional) voice-pack question playback
    listen/     sherpa-onnx facade: Moonshine STT stream + Silero VAD
    format/     candle facade: load 0.5B, run the constrained rewrite
    render/     crossterm: live-edge + settle painter (NO orb)
    pack/       question-pack + model download (behind `download` feature)
  crates/
    talk-core/                pure, dependency-light, wasm-buildable
      questions.rs  selection.rs  slug.rs  frontmatter.rs
      entry.rs      settle.rs     cleanup.rs (policy + prompt, not inference)
      palette.rs    (RUST base tone; cloned from meditate-core)
    talk-wasm/                browser facade (web phase, later)
  questions/                  vendored spine + packs (TOML data)
  web/                        PWA (web phase, later)
```

**The load-bearing boundary:** `talk-core` owns everything *pure* — question
selection, slug derivation, the frontmatter schema, the file-append logic, the
settle state machine, and the cleanup **policy + prompt template** (the
restraint moat). It knows nothing about microphones, ONNX, or Candle. The CLI's
`listen/` and `format/` are thin façades; the web later swaps in `moonshine-js`
+ `transformers.js` façades over the *same* core. This is the meditate-core
split that lets one engine run in a terminal and a browser.

**Engine rationale:** the live jittering edge is the signature feature and
depends on high-quality streaming partials, which sherpa-onnx + Moonshine
deliver natively (plus Silero VAD for phrase boundaries). Candle runs the tiny
pure-Rust formatter LLM with no extra FFI. See `DREAMING.md §6`.

**Reused vs net-new (where the real risk is):** the "mirror" framing applies
only to the *scaffolding* meditate-cli genuinely proves — the workspace split,
the `palette.rs` clone, the `download`-feature gating, `streak`, config, and the
`TerminalGuard` RAII. The hard, novel, unproven third of v1 inherits none of
that confidence and is entirely new: **mic capture** (meditate's cpal is
output-only; input capture + macOS mic permission + sample-rate conversion are
new), **sherpa-onnx streaming partials**, **Silero VAD**, **Candle 0.5B
inference**, and the **settle renderer**. Effort estimates and de-risking spikes
should target these, not the scaffolding. (The cpal mic path also adds a C FFI
build that must coexist with the inherited MSRV 1.82 pin — verify at planning.)

## 6. Commands

```
talk                       # signature: reflect — asks a question, then listens
talk reflect               # explicit reflection (same as bare)
talk journal               # freeform daily — no prompt, just talk
talk "what am I avoiding?" # bring-your-own question
talk unburden | vent       # ephemeral mode — listens, shows, keeps nothing
talk thread [question]     # show a question's accumulated file (the diff);
                           #   no arg → list questions by recency/depth
talk download <pack>       # packs and models (explicit network, like meditate)
talk config [init|path]    # commented config.toml
talk streak                # reflection streak (ported from meditate)
```

Bare **`talk` = reflect**, matching the README hero ("the terminal asks, then
listens").

## 7. The experience (settle)

Rust accent throughout; no orb. Reflect/journal main screen:

```
  talk · reflect                                      ● local · no network

  ┌─ held 3 days ──────────────────────────────────────────────────┐
  │  What are you carrying that isn't yours?                         │
  └─────────────────────────────────────────────────────────────────┘

  The thing I keep coming back to is the weight of other people's      ← SETTLED
  expectations — my mother's voice, mostly.                              (bright, locked)

  um so the part that's actually mine is                               ← LIVE EDGE
                                                                         (dim, jittering)
  ──────────────────────────────────────────────────────────────────
  ● listening  2:14   Light    [space] done · u raw⇄clean · p pause · esc cancel
```

**Settle state machine** (`talk-core/settle.rs`, painted by `render/`):

```
 Live(partial)  ──VAD phrase boundary──►  Committing  ──formatter ~200ms──►  Settled
 dim · rust-grey                          raw leaves edge,                    bright · locked
 jitters as Moonshine                     brief shimmer                       NEVER moves again
 revises hypotheses
```

The one hard rule (`DREAMING.md §5`): **settled text never moves again.** Only
the live edge jitters. Formatting happens *at* commit, so the user watches raw
dissolve and clean appear in its place — never a re-flow. `u` toggles the whole
transcript clean⇄raw (always available because raw is always stored).

> **Settle model amended (2026-06-08, during Plan 2 research).** In the Rust
> toolchain, sherpa-onnx's **Moonshine is offline (non-streaming)** — true
> mid-phrase partials need a streaming Zipformer. We keep Moonshine (§6) and
> adopt the **settle-on-pause** model (`DREAMING.md §5`'s "speak blind, see it
> settled", which the dream flagged as likely *better* for reflection): **Silero
> VAD** detects a speech segment; the dim "live edge" is a calm *listening*
> indicator (not jittering partials); on your pause the segment is transcribed
> and the clean phrase **settles in at once**. The `talk-core` settle machine is
> unchanged — `commit()` fires on the VAD endpoint instead of a partial stream;
> `Live`/`Committing`/`Settled` and the never-re-flow invariant still hold. A
> streaming-Zipformer "live jitter" mode remains a later option behind a flag.

> **Amended again (2026-06-09, Plan 5):** streaming live-jitter is now the default experience (user decision after field testing) — a streaming Zipformer feeds live partials to the edge and endpoints replace VAD; every committed phrase is re-transcribed by Moonshine base (two-pass) and revised in place while committing. Bright = pass-2-final. The settle machine, never-re-flow invariant, and §17 criteria are unchanged; settle-on-pause and the speak-blind texture are retired.

**Rendering invariant that makes "never re-flows" true** (it must be engineered,
not asserted): the settled region is an append-only stack of immutable blocks.
Each committed phrase is laid out once into its *own* block; prior blocks are
never re-wrapped. A clean phrase that differs in length/wrap-count from its raw
form changes only its *own* block's height (a one-time downward shift of the live
edge below it), never the position of settled text above it. The settle test
asserts that re-formatting block *N* leaves blocks *0..N-1* byte-identical in the
render buffer. A full re-layout happens only on a hard terminal resize, never
per-commit.

**Late-revision (commit-lag) window:** streaming STT revises words spoken
seconds earlier, so the Live→Committing promotion is *not* the VAD edge directly
— it is the VAD edge **plus a decode-lag window** (Moonshine's revision horizon,
~300–500ms) during which the phrase stays in `Committing` and may still change.
Only after that window closes does the phrase format and lock. A revision that
arrives for already-`Settled` text is dropped (the settled rule wins) but
recorded to the raw layer so nothing is silently lost; a settle-test fixture
feeds a late revision past the boundary and asserts settled text does not mutate.

**Formatter-latency fallback (the settle must never stall):** `Committing` does
*not* block on the LLM. The phrase settles **immediately** with the
deterministic-Light result (caps/punctuation/filler only — see §10), bright and
locked; the 0.5B rewrite runs async and, *iff* it returns within a short swap
window (~250ms) **and** the diff-guard passes, it replaces that block in place
(still no re-flow, per the invariant). Miss the window or fail the guard and the
deterministic-Light text stays — permanently, no late pop-in. So `~200ms` is a
*target for the async upgrade*, not a blocking path, and it is gated by a
**measurement spike** (0.5B Q4 Light-cleanup latency on a stated hardware floor —
e.g. M1 and a commodity x86 CPU) **before** the engine stack is locked.

**In-session keys:** `space` finish & save · `u` raw⇄clean · `p` pause/resume ·
`esc`/`ctrl-c` cancel (discard, with confirm). Auto-end-on-silence is **off by
default** (configurable) — *you* decide when you're done.

**Ephemeral mode** (`talk unburden`/`vent`) — the inverse:

```
  talk · unburden                    ● local · no network · ✦ nothing saved

  Say it. This keeps nothing.

  I am so angry that I never got to                                    ← live, dim
  ──────────────────────────────────────────────────────────────────
  ● listening  0:48   ✦ ephemeral          [space] release · esc cancel
```

`[space]` → screen clears to **"Released. Nothing was written."** Pure: no
file, no "oops keep it." That purity is the feature — and the one place
zero-network is the entire reason the feature can exist.

**"Keeps nothing" is enforced, not just promised.** Ephemeral is a hard
invariant gating *every* persistence sink, not only the primary file-write:
(1) the §13 crash-recovery/keep-in-memory path is disabled in ephemeral;
(2) the transcript buffer is `zeroize`d on drop and, best-effort, `mlock`ed so
it isn't paged to swap; (3) no temp files, no debug output / crash-dump of
transcript text. A test asserts that after a full ephemeral session **zero bytes
of transcript touch disk** (file, temp, recovery, log). **Threat-model boundary
(stated plainly to the user):** ephemeral means *this tool* writes nothing; it
cannot control your terminal's **scrollback buffer** (the cleared screen still
lives in the emulator) or kernel **swap during hibernation**. The docs say so
and suggest a dedicated / no-scrollback terminal for the most sensitive use.

**The close** (reflect/journal) — provenance + a curated phrase:

```
  → ~/talk/what-am-i-avoiding.md     entry 3 · held 3 days
  "Stillness carries forward."
```

**Eyes-closeable (optional):** if a voice pack is present, the question is
*spoken* (reuse meditate's voice infra) so you can answer with eyes closed and
read it settled afterward. On when a voice pack exists, off otherwise.

**Interaction states (the screens beyond the happy path):**
- **First-run model fetch:** a confirmation screen naming artifacts + sizes
  (Moonshine ~60MB, formatter ~300–400MB) + est. time; a progress state
  (per-file bytes + bar, `esc` cancels); a resumable partial-download state on
  relaunch; a failure state offering retry vs. start-over. `talk download
  models` runs the same flow non-interactively.
- **`talk thread` (no arg):** prints a static list (no live navigation) of
  questions sorted by recency, each line `slug · entries · last`; empty state:
  "No threads yet — run `talk` to start one." With an arg, prints that
  question's file path + tail.
- **Cancel-confirm:** `esc`/`ctrl-c` shows an inline status-line prompt
  `discard this reflection? [y] yes · [n] keep going` — it does **not** reuse
  `space`. Ephemeral skips the confirm (nothing is at risk).
- **Pause:** `p` changes the status line to `⏸ paused …` and **freezes the
  timer**; the listening dot stops pulsing; a present voice pack speaks nothing.
- **`talk download <pack>`:** no arg lists installed + available packs; a name
  reuses the first-run download component. Question packs (TOML) and models are
  distinct namespaces under the one command.
- **BYO question box:** a first-ever BYO question shows header `new` (not
  `held 0 days`); BYO questions are stored and threadable like spine questions.
- **Write-error recovery:** status line offers `retry · copy to clipboard ·
  discard`; after 3 failed retries it auto-offers clipboard export; the close
  phrase is suppressed until the entry is safely out.
- **Config legibility:** `config.toml` documents each cleanup level with a
  one-line example diff so the choice is informed; `auto_end_silence_seconds`
  (default `0` = off) is the silence config key.

## 8. Data model

**Reflect file** — `~/talk/<question-slug>.md`, one per question, sections
append **chronologically**:

```markdown
---
id: avoidance-core           # immutable; binds the thread — never derived from text
question: "What am I avoiding?"
slug: what-am-i-avoiding     # filename, for readability; may change without orphaning
pack: examen                 # spine · byo · <pack name>
addressee: self              # self · future-self · empty-chair · the-critic · …
created: 2026-06-06
entries: 3
last: 2026-06-08
---

## 2026-06-06
<!-- raw: um the thing I keep not saying out loud is… -->
The thing I keep not saying out loud is…

## 2026-06-07
…
```

**Journal file** — `~/talk/2026-06-08.md`, date-keyed, sessions append within
the day (`## 08:14`, `## 21:30`), each with its raw comment.

**File permissions:** `~/talk/` is created `0700` and every file within it
`0600`, enforced programmatically at creation (not left to the user's umask) —
on a shared/multi-user machine the contents are otherwise world-readable, which
a journal of private reflections must never be.

**Raw recovery:** verbatim transcript stored as an HTML comment under each
section — self-contained (one file = everything, portable into any vault),
hidden in Obsidian reading view, read back by `u`. `keep_raw = true` default.
**Data-exposure disclosure (surfaced on first run):** the raw comment is
unredacted plaintext, so pointing `~/talk/` at an Obsidian / cloud-synced vault
sends your verbatim words to that backend (iCloud/Dropbox/etc.) — the "local"
promise covers *this tool*, not where you choose to store its output. Two
mitigations ship: `keep_raw = false` (store only the cleaned text), and an opt-in
**sidecar raw store** (`~/talk/.raw/`, dot-prefixed so vault sync and Obsidian
indexing skip it) for recovery without syncing raw.

**Question identity & slugs** (`talk-core/slug.rs`):
- Pack/spine questions carry an **immutable `id`** — authored, stable, and
  *separate from both the display text and the slug*. The `id` (recorded in
  frontmatter as `id:`), not the wording, is what binds a thread/`held` run to
  its file. This is what survives a contributor rewording a question or fixing a
  typo: edit the text freely and the history never orphans. The filename uses a
  readable slug derived from the id; renaming the slug never moves the thread,
  because the binding is the `id`.
- BYO has no authored id, so it derives a deterministic kebab slug (lowercase,
  strip punctuation, ~6 words/60 chars, short hash on collision) used as both id
  and filename. **Caveat (stated in docs):** a *rephrased* BYO question is a new
  thread — on a near-match `talk` offers a "continue an existing thread?" prompt
  rather than silently diverging.

**Pack schema** — TOML in `questions/`, loaded by `talk-core`:

```toml
# questions/future-self.toml
name = "future-self"
description = "Talk to the you that's coming."

[[questions]]
slug = "what-are-you-scared-of"
text = "Tell the you of next December what you're most scared of."
addressee = "future-self"
cadence = "daily"          # daily rotation, or "held:7" for one question 7 days
```

**Selection** (`talk-core/selection.rs`) — the "aliveness" (selection, not
generation, per `DREAMING.md §3`): picks this session's question from active
pack + **time-of-day** (morning-seed vs evening) + **rotation** (avoid recent
repeats) + **streak-gated depth** + **cadence** (a question mid-`held:7` run
keeps being served until the run completes). Local state only (`state.rs`):
last-served, per-slug count, current held-run.

**Ephemeral:** session flag; the file-write step is skipped entirely;
transcript lives only in memory and is dropped on exit.

## 9. v1 packs

v1 vendors the **65-question spine** + **one flagship pack per direction**; the
remaining packs ship post-launch via `talk download <pack>` (an OSS contribution
surface). This represents all four directions at launch without authoring ~18
packs to depth up front. Packs are mostly *data*; the only new code is
`addressee`, `cadence`, and the ephemeral session flag.

Vendored in v1 in **bold**; the rest are downloads:

- **Address** — talk *to* someone not here: **`future-self`** · then
  `younger-self`, `empty-chair`, `unsent`, `rehearsal`.
- **Inner-dialogue** — talk to a part of you: **`parts`** (IFS) · then
  `the-critic`, `kind` (self-compassion), `pep` (self-coaching).
- **Thinking & examination** — **`examen`** + **`held`** (one question for
  7 days — the thread-builder) · then `decide`, `pages` (brain-dump), `naikan`.
- **Ephemeral** — **`vent` / `unburden`** run in ephemeral mode.

`held:7` cadence ships with the `held` flagship. **Streak-gated depth defers to
v2** — v1 ships streak *display* (ported from meditate) to collect the data, and
gates the behavioral depth logic on it in a follow-on release.

## 10. Formatter & restraint

`talk-core/cleanup.rs` = policy + prompt; `src/format/` = Candle inference.

**Deterministic layer (before the LLM):** spoken commands ("new line", "new
paragraph", "period") → string handling; backtrack ("scratch that" / "actually
no") → remove preceding clause on a >3-word-reduction rule.

**LLM constrained rewrite — levels None · Light · Medium · High:**
- **Light** (reflect default): caps, punctuation, leading filler. Never
  reorders, never substitutes, never "fixes" misheard words.
- **Medium** (journal default): + disfluency removal, light sentence joining.
- **High:** + paragraphing, cue-driven list detection.

**Diff-guard (the moat) — guards *harm*, not *volume*:** a single global
edit-distance is the wrong metric (it rejects a benign long filler-strip while
passing the tiny, catastrophic `"I love her" → "I loathe her"` substitution).
Instead the guard is **content-word preservation**: tokenize input and output,
drop a known filler/disfluency set, then require that every remaining *content
word* in the output also appears in the input (none added, removed, or
substituted beyond the allowed filler set) with order preserved. Any violation
**rejects** the rewrite and falls back to deterministic-Light or raw — fail-safe
is always your words. The checked-in **eval set** makes this *falsifiable*: each
fixture is `(raw input, impermissible-edit assertions and/or a reference
acceptable output)`; the CI score is the fraction of fixtures with zero
impermissible edits; and the suite includes a deliberately over-editing mock
model that **must** score red — so "restraint" is a test that can fail, not a
vibe. Restraint is the discipline (`DREAMING.md §4`).

## 11. Privacy — true by construction

- The session path (`listen` / `format` / `session`) has **zero network
  dependencies**. Downloads live behind a cargo `download` feature (like
  meditate) → core compiles with `--no-default-features` and physically cannot
  phone home. A test asserts the session path links no HTTP.
- `● local · no network` in the status line is therefore simply true.

**Models / first run (the honest asterisk):** Moonshine (~60MB) + formatter
(~300–400MB) are fetched **once**, explicitly (`talk download models` or a
first-run prompt), then cached → offline forever. The privacy claim is about
**use**, not one-time setup, and is framed plainly. **Download integrity is
non-optional:** HTTPS with cert validation, **SHA-256 hashes pinned in the repo**
and verified before a cached model is ever loaded (a mismatch refuses to run),
and a `talk download verify` subcommand that re-checks on demand. The model host
and pinned versions live in a lockfile so a fetch is reproducible — without this,
a host compromise / MITM / cache-poisoning could substitute weights that then
run locally on your most private audio.

## 12. Config & persistence (mirror meditate)

- `talk config init` → fully-commented `config.toml`; zero-config still
  launches. Pins: base dir, default mode, per-mode cleanup level, `keep_raw`,
  auto-end-on-silence, speak-the-question voice, keybindings, default pack.
- Resumes last state; `config.toml` pins override remembered state. `talk
  streak` ported from meditate's `streak.rs`.

## 13. Error handling (fail fast, never lose words)

- No mic / permission denied → clear fix (macOS mic permission).
- Model missing → prompt to fetch; do not silently degrade.
- **Formatter fails/times out → fall back to raw; never block the save.**
- Write error → keep transcript in memory, offer retry. Any **alternate path**
  is constrained to under the configured base dir with the same `0600` perms
  (never a user-typed arbitrary or cloud-synced location); the in-memory buffer
  has a retry timeout after which it is `zeroize`d even if unsaved, with a
  warning. **In ephemeral mode this keep-in-memory recovery is disabled
  entirely** (see §7) — ephemeral never persists, even on error.
- Panic → RAII `TerminalGuard` restores the terminal (meditate keeps
  `panic=unwind` for exactly this).

## 14. Testing

`talk-core` is pure → richly unit-testable:
- Selection (rotation, `held:7` cadence, time-of-day, streak gating).
- Slug determinism + collision handling.
- Frontmatter append round-trip (chronological + raw-comment).
- Deterministic cleanup (spoken commands, backtrack).
- Settle state-machine transitions.
- Ephemeral no-write.
- Diff-guard over-edit rejection (with the eval set).

Façades mocked: canned partials → assert settle states; mock LLM → assert
guard. One recorded-audio integration fixture → file-output snapshot. Mirror
meditate's `tests/` layout.

**Privacy & integrity tests (beyond the link-time check):** a link-time "no HTTP
in the session path" assertion is necessary but not sufficient. Add a
**sandboxed runtime no-egress test** (Linux network namespace / macOS pf rules
in CI) that runs a full session and asserts **zero outbound connections**
(catching raw sockets, DNS, and FFI-side calls the link check misses); an audit
step that re-checks Candle + the audio lib for network/telemetry behavior on
every version bump; a **model-tamper test** (a corrupted checksum must refuse to
load); and the **ephemeral zero-bytes-to-disk test** from §7.

## 15. Inherited ethos constraints (non-negotiable)

- **MIT**, **zero-network during use**, **zero-telemetry**. No account, no key,
  no phone-home.
- Rust single binary; `walktalkmeditate/tap` Homebrew + npm; `talk-core` /
  `talk-wasm` workspace.
- Contemplative, minimalist voice. Slow and chill.

## 16. Open questions deferred to the web spec

- Web architecture: faithful terminal-in-browser (xterm, like meditate) vs a
  real DOM/CSS web UI (better settle, `getUserMedia`). `DREAMING.md §9` leans
  real-web. The settle renders better with CSS.
- The web privacy badge (PerformanceObserver self-monitoring).
- Browser model caching / single-file `.html` / PWA install.

## 17. Acceptance criteria (v1 CLI)

- [ ] `talk` asks a curated question, listens on-device, settles text live, and
      appends to `~/talk/<slug>.md` with frontmatter + raw comment.
- [ ] `talk journal` writes/append `~/talk/YYYY-MM-DD.md` with no prompt.
- [ ] `talk "my question"` derives a deterministic slug and reuses its file.
- [ ] `talk unburden` keeps nothing; "Released" screen; no file touched.
- [ ] `talk thread` surfaces a question's accumulated file.
- [ ] Cleanup levels work; raw is always recoverable via `u`; diff-guard
      rejects over-edits.
- [ ] Core experience compiles with `--no-default-features` and makes no
      network calls; the only network is explicit `talk download`.
- [ ] `held:7` serves one question across 7 days.
- [ ] Terminal restores cleanly on quit and on panic.

## Review decisions (resolved 2026-06-08)

The 2026-06-08 multi-persona review surfaced two scope forks (plus three
dependent concerns). Both were reconsidered and settled:

- **Formatter stays in v1** (§4 #9, §10). The settle redesign (§7) makes
  deterministic-Light the instant, always-present layer and the 0.5B LLM an
  *async enhancement* on top — so the formatter no longer blocks a complete v1
  experience. The ~460MB footprint, the **two-engine stack** (sherpa-onnx +
  Candle), and the **CI eval set** are accepted v1 work (the eval set is made
  falsifiable per §10).
- **Packs ship one flagship per direction** (§9), not all ~18. All four
  directions are represented at launch (`future-self`, `parts`, `examen` +
  `held`, `vent`/`unburden`); the rest grow via `talk download`. **`held:7`
  ships** with the `held` flagship; **streak-gated depth defers to v2** (v1
  collects streak data via display only).

Carried forward as advisory (not blocking, revisit during planning): the
moat-vs-effort balance; designing the **2–3-entry drop-off as the success
case** (not only 7-day completion); the eval set's labeled ground truth (§10);
and the solo-maintainer surface.

