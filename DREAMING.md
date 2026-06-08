# talk-cli — Dreaming Notes

> Pre-spec findings. Captured 2026-06-04. This is the *thinking*, not the plan.
> No code yet. A spec comes later; this is what feeds it.
>
> Full dream archive: `~/.kaijutsu/dreams/4a1d4f109dbc6006-talk-cli-open-source-free-wisprflow-companion-to-m-20260604-134148.md`

---

## 0. One-line

**talk-cli = "the terminal that listens."** The third pillar (Walk · Talk ·
Meditate) in the terminal. You speak a reflection; it transcribes on-device;
the words settle into a quiet file. Sibling to `meditate-cli`.

---

## 1. The fork we resolved (the dream)

The idea started as "an open-source, free Wispr Flow." Interrogated through the
`dream` lenses, it split into **two different products that only share a whisper
pipeline**:

| | **A. Reflection** (chosen) | **B. Dictation** (rejected) |
|---|---|---|
| What | Voice journaling in the terminal | System-wide "type into any app" |
| Transcript goes | Your journal / `~/zettelkasten` | Your cursor, anywhere |
| Fit | On-brand, completes the trio | Off-brand (productivity tool) |
| Field | Under-built | **Crowded** (6+ free OSS clones) |
| Moat (2028) | Ritual + framework + brand | ~0; OS ships dictation natively |
| Latency need | None (you walk away) | <200ms or it feels broken |

**Decision: A, the reflection tool.** Five lenses (honest, fit, status-quo,
time, inverse) converged independently — that convergence is the signal.

**Why not B:** "open-source free Wispr Flow" is the *most* crowded corner of the
space, not a gap. Already shipped, several MIT + fully local + menu-bar:
[OpenWhispr](https://github.com/OpenWhispr/openwhispr),
[open-wispr](https://open-wispr.com/),
[Handy](https://openalternative.co/alternatives/wisprflow),
[VoiceTypr](https://github.com/moinulmoin/voicetypr),
[CustomWispr](https://customwispr.com/),
[FreeFlow](https://github.com/zachlatta/freeflow). A 7th clone competes on
execution against a year head start, in a category the OS is about to eat. The
"CLI" shape also fights dictation (which wants a system-wide accessibility
daemon, not a terminal).

**Honest money note:** Wispr Flow is a ~$12–15/mo business *because* dictation
has rare proven willingness-to-pay. Choosing "free + open-source" forgoes the
one monetizable idea here. That's fine — build it for love and ethos — but don't
let it wear a business costume. (A later, non-compromising revenue seam exists:
the *content flywheel* — speak → transcribe → feed thepilgrimage/podcast/reels
pipeline. Parked, not pitched.)

**Note for later:** options C (shared `talk-core` engine powering both a
reflection front-end now and a dictation front-end later) and D (don't build;
adopt open-wispr + deepen pilgrim-ios) remain on the table if priorities change.

---

## 2. The core loop

```
prompt  →  listen  →  transcribe  →  land  →  close
(a Q)     (push-     (on-device)    (a file)  (a phrase)
          to-talk)
```

The thing that makes it *ours* and not a dictation tool: **the constraint is the
feature.** One question. Your voice. Written somewhere quiet you can't doomscroll.

---

## 3. Question sourcing

"Preset vs LLM" is a false binary. There are **three independent axes**:

- **Source** — who authored the words (human-curated / community / generated)
- **Selection** — how *this* question is picked now (random / rotation /
  context / streak / time-of-day)
- **Adaptation** — does it respond to you (history, yesterday, what you just said)

Most of the "aliveness" comes from **selection**, not generation — exactly how
`meditate-cli`'s intelligence is curated patterns + local state, not AI.

**Load-bearing principle:** *the contemplative value of a question is inversely
proportional to how easily an LLM can generate it.* Ask a model for a reflection
question → fluent, novel, hollow (fortune-cookie depth). The **time lens** agrees:
human-curated = timeless; generated = 2026-quaint by 2028. **So the default must
be human-authored.**

- **Spine:** the **65 curated questions** already in
  [`walktalkmeditate`](../walktalkmeditate) (vendor them / share a data file),
  organized by context (morning seed / evening / solo / walking).
- **LLM's only honest job = the responsive second turn** — a follow-up grounded
  in *what you just said*, the one thing a static bank physically can't do.
  Off by default, 100% local.

**Reframes worth keeping:**
- **Fewer questions, held longer.** Naikan / Examen / lectio divina *repeat*.
  Same question for 7 days; the artifact is the *diff* across the week. Inverts
  the whole "generate enough questions" anxiety.
- **Download packs** — `talk download examen|grief|couples-on-a-walk|founders`.
  `meditate-cli` already has the `download <pack>` architecture. Grows the bank
  infinitely, stays human-authored, becomes an OSS contribution surface.
- **Bring your own** — `talk "what am I avoiding?"`. The bank is just the default.
- **The thread (10x)** — a local model reads your *own past reflections* and
  serves the question that continues your arc. No app does this; only knowable
  from your private journal.
- **Question as voice, not text** — reuse `meditate-cli`'s output voice packs;
  the question is *spoken*, eyes closed, then it listens. The whole pillar goes
  eyes-closeable.
- **Earned depth** — streak (already in meditate-cli) gates the harder questions.

**Default shape:** curated 65 spine → selection does the heavy lifting →
community packs for breadth → local model only for the responsive second turn,
off by default. *Generation at the edges, never at the core.*

---

## 4. The "make it pretty" engine (Wispr Flow, demystified)

Wispr's smart formatting feels like magic; it's a **two-stage pipeline**: fast
STT → a small fast LLM doing a **constrained rewrite**. ([Wispr Smart Formatting
docs](https://docs.wisprflow.ai/articles/5373093536-how-do-i-use-smart-formatting-and-backtrack))

- Scope is deliberately narrow: grammar, punctuation, capitalization, disfluency
  — and it **explicitly does not correct misheard words**. It's a *formatter*,
  not a re-transcriber.
- **List detection** = cue-following ("…*one* ship the cli *two* send the deck"
  → numbered list). Default behavior is **flow into prose; structure only on
  strong evidence.** That's the "continuous paragraph" feel.
- **Backtrack** (disfluency/self-correction removal) = mostly rules: trigger word
  ("actually"/"scratch that") + >3-word reduction. Cheap to replicate.
- **Spoken commands** ("period", "new line") = deterministic string handling, no
  LLM needed.

**The counterintuitive lesson: the hard part is *restraint*, not formatting.**
A naive "clean this up" prompt hits 80% in an afternoon and *over-edits* — silently
changing your meaning (the janky-clone failure mode). 80→99% is a discipline
problem: teach the model to *stop*. Wispr's whole design is restraint —
configurable **Cleanup Levels (Light/Medium/High/None)** + always-recoverable raw
text ("Undo AI edit"). The moat is a good **eval set of your own dictations**
tuned against over-editing.

**The cleanup dial is the sleeper feature** — it lets *one engine* serve both
products: dictation wants Medium/High; **reflection wants Light/None** (preserve
the ums — the ums might be the truth).

**The latency fork (the "free" decision):**
- Cloud-fast BYO key (Groq/Cerebras + 8B) → ~100–200ms, feels instant; not free,
  not zero-network.
- Local (small model) → free + private + on-brand; slower, heavier dependency.

**The contemplative escape hatch:** a reflection you spoke and walked away from
can take **3 full seconds to format perfectly and nobody cares.** Dictation is in
a latency arms race; **reflection dissolves the hardest constraint entirely** —
it can run a slow, local, perfect, free, zero-network formatter and lose nothing.

> Caveat: matching Wispr's *full* feature set (per-app
> [context awareness](https://docs.wisprflow.ai/articles/4678293671-feature-context-awareness),
> types everywhere, instant) means rebuilding the crowded clones — and that
> context-reading is exactly what forces the accessibility-daemon architecture +
> sending screen text to a cloud. Skip it.

---

## 5. The live-transcribe *settle* (the magic, designed)

Desired magic: see text transcribe **on the fly** AND have it become **pretty**.

**Hidden conflict:** live-raw and pretty-final can't coexist on the same text.
Streaming STT emits *jittery partial hypotheses* (raw, changing); prettifying
needs a *complete* utterance (batch). You can't format half a sentence. So it's
**two layers and a transition**, not one stream.

**The settle pattern (a.k.a. crystallize):**

```
   live edge  ▸  "um so the three things are one ship the cli two—"   ← dim, jittering, raw
   ───────────────────────────────────────────────────────────────
   settled    ▸  "The three things are:                              ← bright, locked, pretty
                   1. Ship the CLI
                   2. …"
```

- **Layer 1 — live draft:** words stream in dim/ghosted, jittering as hypotheses
  refine. The "it's listening" dopamine.
- **Layer 2 — settle:** at each phrase boundary (silence/VAD), the formatter
  rewrites the completed phrase and it **animates raw → clean** — "um" dissolves,
  list snaps into structure, grey brightens to final. ~100–300ms behind your voice.

**The one hard rule: settled text never moves again.** Only the live edge jitters.
Re-flowing formatted text = motion-sickness; this is *why* Wispr hides the raw and
reveals final-on-release.

**Contemplative reframe (better than Wispr):** Wispr hides the raw; we show the
**settling itself** — raw speech crystallizing into clean prose, like sediment
settling in still water. The half-second of settling *is* the contemplative beat
("stillness," "settle," "return gently"). Renderable on `meditate-cli`'s existing
crossterm + kitty/Ghostty graphics layer (dim ANSI → bright).

**Honest fork:** wanting to *watch every word appear* is a dictation-brain want.
The most contemplative version may show **no live text at all** — eyes closed, a
calm breathing waveform while you speak, and the clean reflection settles into
view only *after* you finish. Sidesteps the live/pretty conflict entirely.
"Speak blind, see it settled" may beat "watch it crystallize" for reflection.

---

## 6. Engine choice — open-source & lean

### Cactus ([github](https://github.com/cactus-compute/cactus)) — technically ideal, ethically wrong
One SDK doing streaming STT (moonshine/whisper/parakeet) **and** the formatter LLM
(Gemma/Qwen), on-device, <120ms, Rust/C bindings, iOS/Android/macOS/Linux.
**But its [LICENSE](https://raw.githubusercontent.com/cactus-compute/cactus/main/LICENSE)
is source-available proprietary** — *"All Rights Reserved,"* free only under **$2M
funding AND $2M revenue**, auto-terminates above; plus a **cloud-fallback that
routes to their servers** by default and unstated telemetry. For an MIT /
zero-network / "open source, golden rule" ecosystem this is a values mismatch and
would **contaminate talk-cli's license.** ❌ Don't depend on it. (Worth *noticing*
its dual-license model as a monetization pattern for your own ventures, though.)

### The open-source answer is *better*, not a compromise

**[Moonshine](https://github.com/moonshine-ai/moonshine) — MIT, and it's the STT
Cactus wraps.** Use it directly:
- MIT code *and* streaming models. No account/key/cloud/telemetry.
- **Built for the live-transcribe magic** — *"does compute while you're still
  talking… continuously supplies partial text updates."* That's Layer 1 of the
  settle, native. (Whisper can't stream cleanly — fixed 30s windows. Moonshine v2
  uses a sliding-window streaming encoder.
  [paper](https://arxiv.org/html/2602.12241v1))
- **Tiny:** 27MB → 245M params; the 245M beats Whisper-Large-v3 on WER.
  **CPU-only**, no GPU/NPU.
- Cross-platform (Python/Swift/Java/C++, iOS/Android/macOS/Linux/RPi) → could be
  shared with pilgrim-ios / pilgrim-android.

**[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) — Apache-2.0**, the closest
*toolkit* analog to Cactus's speech side: streaming STT + TTS + VAD + diarization,
runs on everything incl. NPUs, 12 language bindings, ~2MB wheels.

### Untangling "llama.cpp feels too large"
Half-true. The *runtime* is lean; the **model weights** are what's big — and
that's a constant across **every** engine (Cactus downloads models too). Fixes:
1. **Pick a tiny formatter model.** Formatting is a constrained rewrite, not
   reasoning — **0.5–1B is plenty** (Qwen2.5-0.5B, Llama-3.2-1B, SmolLM2-360M).
   Q4 ≈ 300–400MB, flies on CPU.
2. **For a Rust CLI, skip C++ entirely** — use **Candle** (HuggingFace, MIT/Apache,
   pure-Rust ML). Runs Whisper *and* small LLMs natively in Rust, Metal-accelerated,
   no FFI. The whole pipeline in one idiomatic dependency.

### Recommended stack, tiered to how "pretty" you want it

| Goal | Stack | Footprint |
|---|---|---|
| **Reflection, raw voice** (Light/None) | **Moonshine alone** — no LLM | tiny binary + ~60MB model, **zero network** |
| **Pretty / formatted** (the settle) | **Moonshine + tiny LLM** (Candle or libllama) | + ~400MB model |
| **Want streaming VAD/diarization** | **sherpa-onnx** | one lean C lib, Rust bindings |

**The callback that ties it together:** §3 says reflection wants **Light/None**
formatting → it may need **no formatter LLM at all** → the entire pillar can ship
as *a tiny Rust binary + a 27–192MB MIT streaming STT model, no LLM, no network,
no telemetry.* The "too large" instinct, followed honestly, lands on the **most
on-brand possible outcome: this thing can be genuinely small.**

**Cost of open-lean vs Cactus:** you wire two pieces yourself (STT + optional
formatter) and don't get free NPU routing (Moonshine is CPU-only). Non-issue for
a desktop/Apple-Silicon CLI; the phone NPU story is pilgrim's, already solved.

---

## 7. Open decisions (for the future spec)

1. **Where the transcript lands** — `~/zettelkasten`? pilgrim-obsidian? plain
   dated markdown? Configurable default. *This is the product* — pick deliberately.
2. **LLM in v1, or not?** Leaning **not** — ship Moonshine-only reflection first,
   Light/None cleanup, raw voice preserved. Formatter is a later, optional, local,
   off-by-default feature.
3. **Which tier is v1?** (See §6 table.) Likely "reflection, raw voice."
4. **Standalone vs chained** — separate `talk` binary *and* a chain off
   `meditate` (breathe → bell → talk) so the ritual is one gesture. `talk-core`
   pure crate mirroring `meditate-core`.
5. **Live settle vs speak-blind** — show live crystallizing text, or calm
   waveform + settle-after? (§5 fork.) Maybe a flag; pick the *default*.
6. **Web parity — possibly v1, not v2** (see §9). Fully-local in-browser is
   confirmed feasible via `moonshine-js` (MIT, client-side, streaming). May be
   the better *first* ship for the reflection audience. Keep `talk-core`
   wasm-friendly regardless.
7. **Question pack format** — how the 65 (and download packs) are stored/shared
   with `walktalkmeditate`.

---

## 8. Ethos constraints (inherited, non-negotiable)

- **MIT**, **zero-network**, **zero-telemetry**. No account, no key, no phone-home.
- Rust single binary; `walktalkmeditate/tap` Homebrew + npm; `talk-core` /
  `talk-wasm` workspace pattern.
- Contemplative, minimalist voice. Privacy is the feature. Slow and chill.

---

## 9. Web-local target (no server, in-browser) — feasible, maybe v1

**Question:** a web page that's fully local — nothing touches any server —
transcribes in-browser and saves the text somewhere?

**Answer: yes, confirmed.** This is the `talk-wasm` target (mirrors
`meditate-core`/`meditate-wasm` → cli.pilgrimapp.org), and it may be the better
*first* ship for the reflection audience.

**The stack (zero backend):**

| Need | Browser capability |
|---|---|
| Capture voice | `getUserMedia` / Web Audio |
| Transcribe on-device | **[moonshine-js](https://github.com/moonshine-ai/moonshine-js)** (MIT, client-side, streaming) |
| Live + settle | its two callbacks (below) |
| Save text | File System Access API / download / IndexedDB |

**moonshine-js maps 1:1 onto the §5 settle design:**
```js
new Moonshine.MicrophoneTranscriber("model/tiny", {
  onTranscriptionUpdated(text)  { /* Layer 1: live draft, jittering  */ },
  onTranscriptionCommitted(text){ /* Layer 2: phrase boundary → settle */ },
}).start();
```
*"All audio processing happens locally in the user's web browser — no cloud."*
English model + code are MIT.

**Saving (the one real wrinkle), tiered by browser:**
- **Chrome/Edge/Brave — magic path:** **File System Access API** → user picks
  their real `~/zettelkasten` folder once, page appends to actual files,
  permission remembered. The seamless "lands in your journal."
- **Safari/Firefox — fallback:** download-to-Downloads (Blob) or IndexedDB/OPFS
  + export. Works, less seamless.
- **Universal:** copy-to-clipboard.

**"Nothing touches a server" — precise:**
- **During use: genuinely zero network.** Verifiable: open DevTools → Network
  tab → watch the silence. A *verifiable* trust property, deeply on-ethos.
- **One-time:** page + model (~27MB tiny English) fetched once, then cached
  (service worker / Cache API) → fully offline after; installable as a **PWA**
  or shippable as a single self-contained `.html`. Honest phrasing: "local app
  you fetch once," not "never touches the network."
- ⚠️ **Trap:** the built-in browser `SpeechRecognition` API is **NOT local**
  (Chrome ships audio to Google). Must use the moonshine-js/WASM model.

**Why web-local may beat the CLI as v1:**
- Zero install — a URL or one HTML file vs `brew install`. Reaches the
  non-terminal pilgrims (the actual walktalkmeditate audience).
- The **settle renders better** in a browser (CSS transitions, web fonts,
  smooth dim→bright) than in a terminal.
- A web page is **a tab you deliberately open, not a daemon** — perfectly fits
  "reflection = a deliberate visit," and reinforces reflection-not-dictation.
- Per §6: reflection wants Light/None → maybe **no LLM** → the browser version
  can be *just* moonshine-js → text → save. (Optional prettify later via
  Transformers.js, a tiny LLM over WebGPU, client-side.)

CLI stays the "where developers already work" surface; `talk-core` compiles to
both. For reach + shareability, **web-local is arguably the smarter first ship.**

---

## Sources
- [Wispr Smart Formatting & Backtrack](https://docs.wisprflow.ai/articles/5373093536-how-do-i-use-smart-formatting-and-backtrack)
- [Wispr Context Awareness](https://docs.wisprflow.ai/articles/4678293671-feature-context-awareness)
- [Moonshine (MIT)](https://github.com/moonshine-ai/moonshine) · [moonshine-js (in-browser)](https://github.com/moonshine-ai/moonshine-js) · [v2 streaming paper](https://arxiv.org/html/2602.12241v1) · [Voice announcement](https://huggingface.co/blog/UsefulSensors/announcing-moonshine-voice)
- [sherpa-onnx (Apache)](https://github.com/k2-fsa/sherpa-onnx)
- [Cactus](https://github.com/cactus-compute/cactus) · [LICENSE](https://raw.githubusercontent.com/cactus-compute/cactus/main/LICENSE)
- [OSS Wispr alternatives](https://openalternative.co/alternatives/wisprflow) · [2026 OSS STT benchmarks](https://northflank.com/blog/best-open-source-speech-to-text-stt-model-in-2026-benchmarks)
