---
date: 2026-06-13
topic: talk-web
---

# talk for the web (talk.pilgrimapp.org)

## Summary

A fully functional, privacy-first web version of `talk` at `talk.pilgrimapp.org`: open it,
grant the mic, and it transcribes your spoken reflections **in the browser** — and after a
one-time model download, nothing you say or write ever leaves your machine. It shows talk's
signature live edge jittering and settling into final text, runs the same two-pass pipeline
as the CLI (streaming Zipformer edge + Whisper base.en finalize) via sherpa-onnx's WASM
build, wears talk's **rust** palette, adopts the meditate-web feel (terminal REPL, mobile
chip bar, install funnel, OG card), opens **reflect-first** with a written question, keeps a
**browser-local journal** you return to, and lets you **save/export** any entry as markdown.

---

## Problem Frame

`talk` is the third pillar of Walk · Talk · Meditate, and its sibling `meditate` already
has a web home at `cli.pilgrimapp.org` — a true functional port where a visitor actually
breathes with it in the browser. `talk` has no such front door. Today, the only way to
experience talk is to `brew install` or `cargo install` a Rust CLI and compile an on-device
speech stack — a real barrier for a curious person who heard about talk and just wants to
feel what "speak a reflection and watch it settle" is like. The pillar is incomplete: walk
and meditate meet people on the web, talk does not.

The tension that makes this non-trivial: meditate's engine (breathing math) is tiny and
trivially runs in WASM, so its web port was nearly free. talk's engine is **on-device
speech recognition**, and talk's entire identity is "transcribed entirely on-device, zero
network calls." A web version has to honor that promise — which rules out the easy paths
(cloud STT, the browser's Web Speech API) and means the real work is putting a genuine ASR
pipeline, and the signature live edge, inside the browser. It also means the privacy promise
acquires one honest asterisk the CLI words around with an explicit `talk download` step: the
models must be fetched over the network once, on first visit, before anything runs locally.

---

## Actors

- A1. First-time visitor: arrives never having used talk; meets the model-download gate and
  the first reflect prompt; the audience the install funnel is for.
- A2. Returning reflector: has used the web before; models are cached; returns to continue a
  thread or read past entries.
- A3. Existing talk CLI user: already journals with the CLI; the web is a secondary surface;
  cares about parity and about moving entries into their real `~/talk`/Obsidian via export.

---

## Key Flows

- F1. First visit, permission & model download
  - **Trigger:** A1 opens `talk.pilgrimapp.org` for the first time.
  - **Actors:** A1
  - **Steps:** Page paints instantly in the rust theme; a "taste it first" preview lets the
    visitor feel the live edge settling before committing; talk explains it runs on-device
    and needs a one-time ~330 MB download; on accept, download begins with clear progress
    (bytes + %) and a cancel option while the preview keeps playing; each artifact is
    checksum-verified before caching; mic permission is requested with rationale, and every
    permission outcome has a defined screen.
  - **Outcome:** Verified models are cached locally and the visitor lands in the reflect door.
  - **Covered by:** R7, R8, R9, R10, R11, R12, R14

- F2. Reflect session (default front door)
  - **Trigger:** Mic ready; visitor begins.
  - **Actors:** A1, A2
  - **Steps:** talk presents a curated question as on-screen text; the visitor answers aloud;
    the live edge shows dim partial text that updates as they speak; on pause each phrase
    settles into bright final text (Whisper base.en); controls mirror the CLI (done · pause/
    off-record · raw⇄clean · cancel); the entry lands in the local journal under that
    question's thread.
  - **Outcome:** A reflection is captured, cleaned, and stored locally; answers to the same
    question accumulate into a returnable thread.
  - **Covered by:** R2, R3, R4, R14, R18, R24, R25

- F3. Journal session
  - **Trigger:** Visitor chooses journal (freeform, no question).
  - **Actors:** A1, A2
  - **Steps:** Visitor speaks freely; live edge settles; entry appends under today's date.
  - **Outcome:** A freeform entry is stored in the local journal.
  - **Covered by:** R15, R18, R24

- F4. Unburden session (ephemeral)
  - **Trigger:** Visitor chooses unburden.
  - **Actors:** A1, A2
  - **Steps:** Visitor speaks; transcription shows on screen; on session end nothing is
    written, a brief closure moment plays, and the transcript buffers are wiped.
  - **Outcome:** Nothing is kept; the visitor has said it and let it go.
  - **Covered by:** R16

- F5. Return, re-read, and export
  - **Trigger:** A2/A3 returns or finishes an entry.
  - **Actors:** A2, A3
  - **Steps:** Models load from cache (no download); the journal view lets the visitor browse
    past threads/entries; any entry can be downloaded as markdown or copied to clipboard.
  - **Outcome:** The visitor re-reads their history and moves entries into `~/talk`/Obsidian.
  - **Covered by:** R11, R19, R20, R21

---

## Requirements

**On-device transcription & privacy**
- R1. Audio capture and transcription happen in-browser; after the one-time model download,
  there are zero network calls and no audio or text ever leaves the device.
- R2. The pipeline mirrors the CLI's two passes — a streaming Zipformer live edge plus a
  Whisper base.en finalizer — running in sherpa-onnx's WebAssembly build via JS bindings.
- R3. The live edge renders as dim partial text that updates while speaking and settles into
  bright final text on pause. Its idle behavior is also defined: (a) extended silence without
  a pause, (b) the user ending a session while a partial is still dim (finalize or drop),
  and (c) a long no-token interval — so the live edge never reads as a hang.
- R4. Session controls mirror the CLI: done, pause/off-record (nothing said while paused is
  kept), toggle raw ⇄ clean, and cancel — keyboard-driven on desktop, and present in the
  chip bar on mobile.
- R5. Privacy is asserted up front and worded precisely — "after a one-time model download,
  nothing you say or write leaves your browser" — stated once, without nagging.
- R6. Zero-egress is enforced and verifiable: all fonts, scripts, and assets are self-hosted
  (no third-party CDNs, analytics, or error reporting), under a strict Content-Security-Policy
  that permits no external connections beyond the model host.
- R7. Model integrity: artifacts are served over HTTPS from a named host, each verified
  against a pinned SHA-256 before being committed to cache and re-verified on load; a failed
  check refuses the model rather than running it.

**First run, permissions & models**
- R8. The first visit requests mic permission with a clear rationale and defines every
  permission state — pending (browser dialog open), granted, denied (with browser-specific
  re-enable guidance and a retry), denied-and-suppressed (no dialog appears), and
  device-unavailable (mic in use / hardware error) — each with its own screen, never a dead
  end.
- R9. The first visit presents a one-time model download (~330 MB) with clear progress (bytes
  and %), a cancel affordance, and defined sub-states: pre-accept, downloading,
  download-complete-before-mic, and mid-download navigate-away/return.
- R10. A genuine "taste it first" preview lets a visitor feel the live edge settling before
  committing to the download, and keeps playing during the download so the wait conveys the
  talk experience rather than a dead spinner.
- R11. Models are cached in-browser after first download; subsequent visits skip the download
  and start immediately.
- R12. Failed, partial, or interrupted downloads recover gracefully (retriable/resumable),
  never leave a corrupt model, and re-download cleanly if the cache is later evicted.
- R13. Offline states are defined: models-cached + offline works normally (with a subtle
  local-only indicator); not-cached + offline shows a clear "connect once to download"
  blocked state; mid-download network loss pauses and resumes on reconnect.

**Modes & the reflect prompt**
- R14. Reflect is the front door: it presents a curated question (reusing the CLI's question
  set) as on-screen text — no text-to-speech — then listens; answers to the same question
  accumulate into a returnable thread. How the next question is chosen and whether the visitor
  can skip/re-roll are defined.
- R15. Journal mode offers a freeform entry with no question, appended under today's date.
- R16. Unburden mode is ephemeral: it transcribes and shows, keeps nothing, plays a brief
  closure moment, and wipes its transcript buffers on session end.
- R17. Mode-switching is defined: whether switching is allowed mid-session or only between
  sessions, what happens to in-progress text on switch, and a confirmation when leaving
  unburden mid-session (nothing will be kept).

**The journal: persistence, navigation & export**
- R18. Reflect and journal entries persist in a private, browser-local journal the visitor can
  return to and re-read; nothing is sent anywhere.
- R19. The journal view has a defined information architecture: how entries are organized (by
  date and by reflect-question/thread), its empty state, and what "continue a thread" does
  mechanically.
- R20. Any entry can be saved/exported as markdown — file download (primary) and
  copy-to-clipboard (secondary, with a one-time note that the clipboard is shared with the OS
  and installed extensions) — for dropping into `~/talk` or Obsidian; the export entry point
  and post-export confirmation are defined.
- R21. Because storage is per-browser and device-local, the product is honest about
  durability: on the first kept entry it surfaces export as the way to keep words for good,
  detects private/incognito mode and warns that nothing will persist, and flags eviction
  risk. The "returnable thread" framing is scoped to this browser.
- R22. At-rest exposure on shared devices is addressed: a clear warning that entries are
  stored unencrypted in this browser profile and are readable by anyone using it. Optional
  passphrase encryption is noted as a later enhancement (see Scope Boundaries).
- R23. Storage-failure states are defined — quota exceeded, storage-persistence denied, and
  write failure — each surfaced to the user rather than failing silently.
- R24. Cleanup parity with the CLI: the clean/raw toggle and deterministic paragraphs apply
  (journal defaults to high). The personal lexicon is deferred (see Scope Boundaries).

**Look, feel & reach**
- R25. The site carries talk's **rust** palette and its themes (rust default, high-contrast,
  mono) — not meditate's moss.
- R26. The site adopts meditate-web's aesthetic and structure: terminal REPL feel, boot
  banner, an OG social card, and a soft funnel to `brew install talk`, served at
  `talk.pilgrimapp.org`.
- R27. Mobile is a v1 goal: mic capture plus a touch chip bar (which carries on mobile the
  controls that are keyboard-driven on desktop), responsive layout, and safe-area awareness.
  Mobile shipping is contingent on the early latency validation: if Whisper base.en finalize
  cannot hit the latency bar on target phones, a lighter mobile finalizer (Moonshine) ships
  for mobile in v1 rather than slipping mobile out of the release.

---

## Acceptance Examples

- AE1. **Covers R2, R3.** Given the mic is active, when the visitor speaks, then partial text
  appears dim and updates live; when they pause, that phrase settles into bright final text.
- AE2. **Covers R4.** Given a live session, when the visitor pauses (off-record), then nothing
  spoken while paused is kept; resuming continues capture.
- AE3. **Covers R1, R5, R6.** Given any session after models are cached, when transcription
  runs, then the network panel shows no requests at all — no audio, no text, and no
  third-party asset or analytics call.
- AE4. **Covers R8.** Given the visitor denies mic permission, when they try to start, then a
  clear message explains talk needs the mic and how to re-enable it, with no crash; if the
  browser suppresses the re-prompt, the screen still guides them rather than appearing to hang.
- AE5. **Covers R11.** Given models were downloaded on a prior visit, when the visitor returns,
  then no download occurs and they can speak immediately.
- AE6. **Covers R12.** Given a download interrupted midway, when the visitor retries, then it
  resumes or restarts cleanly without leaving a corrupt model.
- AE7. **Covers R7.** Given a cached model whose checksum no longer matches on load, when the
  app starts, then it refuses the model and re-downloads rather than running tampered weights.
- AE8. **Covers R13.** Given cached models and no network, when the visitor opens the app, then
  it works normally with a subtle local-only indicator; given no cache and no network, it
  shows a "connect once to download" blocked state.
- AE9. **Covers R16.** Given unburden mode, when the session ends (done or tab closed), then no
  entry is stored, a brief closure moment plays, and the transcript buffers are wiped.
- AE10. **Covers R20, R21.** Given a stored entry, when the visitor exports it, then they
  receive a markdown file (and a copy option) matching what they would keep in `~/talk`; and
  on the first kept entry, the app surfaces export as the way to keep words beyond this browser.
- AE11. **Covers R21.** Given a private/incognito window, when the visitor keeps an entry, then
  the app warns that nothing will persist after the window closes.

---

## Success Criteria

- A first-time visitor goes from landing → tasting the preview → granting mic → speaking →
  watching words settle → saving a markdown file, without reading any docs.
- A returning visitor re-opens, skips the download entirely, and re-reads or continues a past
  thread.
- Privacy is verifiable: after the model download completes, a browser network panel shows
  zero further requests — no audio/text egress and no third-party calls.
- The experience is recognizably *talk* — rust palette, the live edge, a reflect question —
  not a generic dictation widget.
- A visitor on a shared or temporary browser is never surprised by lost or exposed entries:
  export and the durability/exposure warnings make the local-only model honest.
- Downstream: `ce-plan` can build without inventing modes, persistence behavior, interaction
  states, the engine choice, or the privacy contract.

---

## Scope Boundaries

- Personal lexicon corrections — deferred to a later version (clean/raw toggle and
  deterministic paragraphs still ship in v1).
- Optional at-rest passphrase encryption of stored entries — deferred; v1 ships the
  shared-device warning (R22) instead.
- A WebGPU-accelerated desktop tier (transformers.js / Moonshine Streaming) — deferred. Note
  the related-but-separate mobile fallback in R27: a lighter Moonshine finalizer is in v1
  scope *if* the latency spike shows Whisper base.en can't keep up on phones.
- Accounts, cloud sync, cross-device journal sync, any server backend — out; off-ethos for
  talk. Markdown export is the only bridge off the device.
- Web Speech API or any cloud STT — rejected outright; it would break the privacy promise.
- Audio soundscapes / voice-guide (meditate-web features) — not applicable to talk.
- Importing existing CLI `~/talk` entries into the web journal — out; export is one-way
  (web → disk).
- The talk CLI itself — unchanged; choosing Whisper base.en for the web does not modify it.

---

## Key Decisions

- Real on-device, in-browser transcription (not a scripted demo, not cloud STT): preserves
  talk's privacy ethos and matches meditate-web's "actually use it" parity.
- Engine = sherpa-onnx's Emscripten/WebAssembly build, accessed through its JS bindings. This
  is the same models and algorithms as the CLI (algorithmic parity) but **not** the Rust
  `sherpa-onnx` crate the CLI links — that crate cannot target wasm32, and the CLI's Rust
  listen pipeline does not cross to WASM. The browser integration is a fresh JS/TS binding
  against the wasm module.
- Finalizer = Whisper base.en, chosen for **accuracy and self-punctuation/casing** — the same
  reason the CLI itself adopted base.en after a smaller model produced word-substitution
  errors and no punctuation. CLI consistency is a secondary nicety, not the primary reason.
  The genuinely live alternative is a lighter mobile finalizer (Moonshine) / WebGPU tier,
  which is the mobile fallback (R27), not "Moonshine Tiny."
- Cleanup parity via compiling `talk-core`'s deterministic cleanup to WASM (mirroring how
  `meditate-core` compiles its pure Rust to WASM), avoiding a drift-prone TypeScript
  reimplementation. A TS reimplementation is the fallback only if the WASM façade proves
  impractical.
- v1 targets the single-threaded SIMD WASM path → no SharedArrayBuffer, therefore no
  COOP/COEP cross-origin isolation, therefore static hosting (GitHub Pages, like meditate-web)
  works. If threads prove necessary for acceptable latency, hosting must supply COOP/COEP (a
  header shim or a host that allows custom headers); the early latency spike decides this.
- Self-host all assets under a strict CSP so the zero-egress promise is enforceable and
  verifiable, not just asserted.
- Browser-local persistence only, with markdown export as the durability bridge — and explicit
  loss/exposure warnings (R21, R22), because the storage substrate alone cannot guarantee the
  "returnable thread" the product promises.
- Reflect-first front door, presenting the question as text (no TTS, matching the CLI).
- Retain talk's rust palette (not meditate's moss): brand identity across the pillar.

---

## Dependencies / Assumptions

- True one-time download footprint is **~330 MB** — the CLI's measured pair (streaming
  Zipformer ≈128 MB + Whisper base ≈199 MB). WASM int8 quantization may shift this; the
  figure must be validated early because the funnel premise and the storage budget depend on
  it. (The earlier ~180–220 MB figure was a Moonshine-based estimate and did not survive the
  Whisper-base.en parity decision.)
- Mobile real-time factor for Whisper base.en finalize in iOS Safari / Android Chrome is the
  make-or-break unknown; validate early, with the Moonshine mobile finalizer (R27) as the
  shipping fallback.
- A browser storage mechanism (Cache API or OPFS) can persist ~330 MB across sessions with
  storage-persistence opt-in; iOS Safari eviction is the binding risk, and eviction triggers a
  clean re-download (R12).
- Audio capture via `getUserMedia` at 16 kHz mono through an AudioWorklet; the iOS Safari
  audio-routing quirk must be handled.
- Adoption risk: a ~330 MB first-visit gate reintroduces a barrier of the same *kind* the
  project exists to remove (just relocated from the terminal to the browser). The "taste it
  first" preview (R10) is the mitigation, and bounce-at-the-gate is a named product risk to
  watch.

---

## Outstanding Questions

### Resolve Before Planning

- None — the product scope is settled. The validation items below are measurable facts, not
  product decisions, so they are sequenced as early-planning gates rather than blockers.

### Deferred to Planning

- [Affects R9, R11][Needs research · validate early] Confirm the true WASM download footprint
  (≈330 MB baseline) and final quantization; the first-run ritual and storage budget depend
  on it.
- [Affects R27][Needs research · validate early] Mobile real-time factor for Whisper base.en
  finalize, and the latency bar below which the Moonshine mobile finalizer ships instead.
- [Affects R6, R7][Technical] Final model host and CSP allowlist — self-host on the
  `talk.pilgrimapp.org` origin vs a pinned CDN with Subresource Integrity.
- [Affects R2][Technical] Confirm the single-threaded SIMD path is fast enough (no COOP/COEP);
  if not, resolve the hosting-headers question.
- [Affects R24][Technical] Confirm the `talk-core` → WASM cleanup façade is viable (preferred
  over a TypeScript reimplementation).
- [Affects R16][User decision] Whether unburden stays a distinct v1 mode or slips to v1.1 —
  it is the lightest-weight mode (journal minus persistence) and the cheapest to add later.
- [Affects R18][Technical] Deep-link sharing shape — share a reflection *question* only (e.g.
  `#q=...`); private words must never leave the device.
