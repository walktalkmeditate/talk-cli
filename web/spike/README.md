# talk-web U1 spike harness

**THROWAWAY.** This page exists only to answer the load-bearing questions that gate the whole
talk-web build (plan §U1, "ASR feasibility, footprint & CSP-loadability spike"). It is **not part
of the shipped app**, it does **not import from `web/src/`**, and it must never leak into it. Once
the go/no-go memo is recorded in the plan's Open Questions and the build branch is chosen, delete
`web/spike/`.

It is a **measurement scaffold**, not a working demo. The instrumentation is real. The sherpa-onnx
ASR integration is a **seam you complete** by dropping in the real Emscripten loader + models —
search `spike.ts` for `WIRE:` to find every seam. Nothing here fabricates a measurement: every
value that depends on the (unwired) recognizer reads `seam not wired` until you wire it.

---

## ⚠ Harness limitations — read before trusting any number

These bound what the spike can prove. They are surfaced loudly in the results panel and the copied
memo as well.

- **Metrics 6 & 7 (module shape + zero-egress) are MAIN-THREAD ONLY.** The `fetch`/`XHR` wrappers
  patch only the page's `fetch`/`XHR`. A **threaded** sherpa build loads its WASM + pthread workers
  via worker/pthread fetches the page **cannot** intercept — those requests are invisible here.
  **Confirm module shape AND egress for a threaded build in the browser Network panel** (or a logging
  proxy), not from this harness.
- **`crossOriginIsolated` can't be enabled via meta-CSP.** A `<meta>` tag cannot set COOP/COEP, so
  the threading branch (*single-threaded OK* vs *threads needed*) must be confirmed on a real
  **header-setting host**, not concluded from this spike.
- **The chunked digest (metric 3) is I/O-only.** It runs a full SHA-256 per slice (placeholder), so
  its total ≈ the full-buffer cost. It is **not** numerically comparable to the full-buffer hash —
  do **not** conclude "chunking isn't worth it" from it. Wire a streaming SHA-256 (e.g. `hash-wasm`)
  for the true chunked cost.
- **Footprint (metric 2) is baseline-subtracted but origin-wide.** A pre-load `storage.estimate()`
  baseline is subtracted so the reported delta ≈ the model cache, but `estimate()` counts everything
  this origin has cached, not just the models.
- **The capture buffer is capped at ~60 s.** To avoid OOM on the mid-range phone this harness
  targets, the rolling mic buffer evicts the oldest frames past ~60 s (logged once). "Finalize now"
  therefore finalizes only the last ~60 s of audio.

---

## What it measures (one row per metric, all surfaced in the results panel)

| # | Metric | How the harness gets it |
|---|---|---|
| 1 | **RTF** (real-time factor) — live edge + base.en finalize | wall-clock compute ÷ audio duration, via `RtfMeter`; you call it from the sherpa callbacks at the `WIRE:` seams |
| 2 | **True cached footprint** | `navigator.storage.estimate()` → used/quota bytes; flags against the ~330 MB envelope |
| 3 | **`crypto.subtle.digest` cost** on the ~199 MB file | full-buffer digest vs chunked slice walk; tells you if mobile needs streaming hashing |
| 4 | **CSP directives the loader violates** | draft strict CSP in the `<meta>` + a `securitypolicyviolation` listener logging every blocked directive/URI |
| 5 | **Single-threaded vs threads** | reports `crossOriginIsolated`, `SharedArrayBuffer`, threads-requested, ran-single-threaded |
| 6 | **Module/asset shape** | a `fetch`/`XHR` wrapper logs every filename + count; you record ArrayBuffer-vs-Emscripten-FS at the seam |
| 7 | **Zero-egress sanity** | "Mark models loaded" → every request after that is flagged as an egress candidate (a mini precursor to U10's canary) |

---

## (a) What this is — and that it's throwaway

See above. It lives in `web/spike/`, is independent of the Vite app (`web/tsconfig.json` only
includes `src`, so this TS is never in the app build), and gets deleted after U1 closes.

---

## (b) Setup: obtain the sherpa-onnx WASM assets, place them, serve the page

### 1. Get the sherpa-onnx WASM ASR build + models (you supply these — ~330 MB, not vendored)

The harness does **not** download or vendor sherpa-onnx. You supply:

1. **The Emscripten loader** (`.wasm` + the generated `.js` glue, e.g. `sherpa-onnx-wasm-main-asr.js`
   and `sherpa-onnx-asr.js`). Build it from the sherpa-onnx repo's `wasm/asr` target, or copy the
   prebuilt loader from a k2-fsa Hugging Face Space (below).
2. **Streaming Zipformer EN** model (the live-edge recognizer).
3. **Whisper base.en (int8)** model (the finalizer).
4. **Moonshine tiny** model (for the mobile-fallback RTF comparison — metric 1 on phone).

**Reference loaders to copy** (open the Space, inspect/download its assets — same loader pattern):

- Streaming Zipformer EN:
  `https://huggingface.co/spaces/k2-fsa/web-assembly-asr-sherpa-onnx-en`
- VAD + Moonshine (the mobile-fallback comparison):
  `https://huggingface.co/spaces/k2-fsa/web-assembly-vad-asr-sherpa-onnx-moonshine`

Build/model docs: the sherpa-onnx project (`k2-fsa/sherpa-onnx`, `wasm/` dir) and its
`sherpa-onnx-wasm-*` release archives.

### 2. Place the assets next to this page

Put the loader `.js`/`.wasm` and the model files under `web/spike/` (or a subfolder like
`web/spike/assets/`). Keep filenames the loader expects. **Do not commit them** — they are large and
this whole folder is throwaway. Then:

- In `spike.ts`, complete the `WIRE:` seams: import/instantiate the loader, create the streaming +
  offline recognizers, call `liveRtf.record(...)` / `finalRtf.record(...)`, set `partialsEl` /
  `finalEl`, and fill `results.assetShape` / `results.threadsRequested` / `results.ranSingleThreaded`.
- For the chunked-digest true number (metric 3), wire a streaming SHA-256 (e.g. `hash-wasm`) into the
  marked chunk loop; the placeholder times the I/O shape only.

### 3. Serve it so the CSP behaves like GitHub Pages

The draft strict CSP is applied via `<meta http-equiv>`. For the directives U1 cares about
(`script-src`, `worker-src`, `connect-src`, `wasm-unsafe-eval`), meta-CSP ≈ header-CSP — good enough
to learn the required set. But **mic capture needs a secure context** (HTTPS or `localhost`).

- **Local (quick):** any static server over `localhost` (a secure context), e.g.
  `python3 -m http.server` from `web/spike/`, or `npx http-server web/spike`. Open
  `http://localhost:<port>/`. `getUserMedia`, `crypto.subtle`, and `storage.estimate` all work on
  `localhost`.
- **Closest to production (do this for the final reading):** push `web/spike/` to a **GitHub Pages
  preview** and load over HTTPS. That confirms the loader behaves under a real static host with the
  meta-CSP, with no COOP/COEP headers (the single-threaded assumption U1 is testing). If U1 finds
  threads are required, that's the host-migration branch — note it in the memo.
- **CSP relax toggle:** the panel has a "relax CSP" checkbox. Caveat: browsers **intersect**
  multiple CSPs, so a strict meta already applied at parse time cannot be loosened by injecting a
  second one. To truly confirm "does the loader run at all," edit the strict `<meta>` in
  `index.html` to the permissive policy (or serve a permissive header), reload, confirm it runs,
  then tighten directive-by-directive watching the violation log.

> **Phone access:** to run on a real mid-range phone, the GitHub Pages preview (HTTPS) is simplest.
> For localhost-over-LAN you need an HTTPS tunnel (mic requires a secure context off `localhost`).

---

## (c) Measurement checklist — run on BOTH a desktop AND a mid-range phone

Run every row on **desktop** and on **a mid-range phone in both Android Chrome and iOS Safari
16.4+**. Record the numbers in the memo (the "Copy results memo" button pre-fills the auto-captured
ones; you fill the `[HUMAN]` lines).

| # | Metric | Action in the harness | Record |
|---|---|---|---|
| 1a | RTF live edge | Start mic, speak ~30 s, watch `RTF — live edge` | RTF (target < 1.0, ideally < 0.5) |
| 1b | RTF finalize (base.en) | Stop / "Finalize now", watch `RTF — finalize` | RTF, desktop + phone |
| 1c | RTF finalize (Moonshine) | Re-wire the offline recognizer to Moonshine, repeat 1b on phone | RTF — is it acceptable where base.en isn't? |
| 2 | Cached footprint | Load all models, click "Measure footprint" | bytes used; matches ~330 MB? |
| 3a | digest full-buffer | "Time digest…", pick the ~199 MB file | ms; did it OOM/fail on phone? |
| 3b | digest chunked **(I/O-only, NOT comparable to 3a)** | (same run reports both) | ms; I/O shape only — wire a streaming SHA for the real number |
| 4 | CSP violations | Load the recognizer; read the violation rows in the log | exact directives/URIs blocked |
| 5a | crossOriginIsolated / SAB | read on boot (top of metrics) | true/false |
| 5b | threads / single-threaded | after a finalize, read `threads requested` / `ran single-threaded` | yes/no |
| 6 | module/asset shape **(MAIN-THREAD ONLY)** | "Mark models loaded" then read the module-count + names in the log; **also check the Network panel for worker/pthread fetches** | count, filenames, ArrayBuffer vs FS |
| 7 | zero-egress **(MAIN-THREAD ONLY)** | after "Mark models loaded", do a full recognition, scan the log for "EGRESS CANDIDATE" rows; **also check the Network panel for worker fetches** | clean? anything beyond `connect-src`? |

Then click **"Copy results memo"** and paste the result into the plan's Open Questions.

---

## (d) Go/no-go memo template (structured to feed the plan's decision tree)

The "Copy results memo" button generates this with the auto-captured numbers filled in; you complete
the `[HUMAN]` lines from your desktop + phone runs. The decision-tree rows below mirror the plan's
"Risk Analysis & Mitigation — U1 spike decision tree".

```
## U1 spike go/no-go memo  (paste into the plan's Open Questions)

### Raw measurements
- RTF live edge (streaming Zipformer):           ____
- RTF finalize base.en  — desktop / phone:        ____ / ____
- RTF finalize Moonshine — phone:                 ____
- Cached footprint Δ (storage.estimate, baseline-subtracted, origin-wide): ____ bytes  (matches ~330 MB? __)
- digest full-buffer / chunked I/O-only (~199 MB file): ____ ms / ____ ms  (chunked NOT comparable; mobile OOM? __)
- crossOriginIsolated (can't enable via meta-CSP) / SharedArrayBuffer: ____ / ____
- threads requested / ran single-threaded (confirm on header host): ____ / ____
- modules fetched (count + names, MAIN-THREAD ONLY):  ____
- asset consumption (ArrayBuffer vs Emscripten FS): ____
- CSP violations caught (exact directives):       ____
- post-"models loaded" egress beyond connect-src (MAIN-THREAD ONLY — check Network panel for workers): ____

### Decision-tree answers
1. base.en fast enough on DESKTOP single-threaded?   [ yes / no ]
2. base.en fast enough on MID-RANGE PHONE?           [ yes / no ]
   - if no, Moonshine tiny meets the bar on phone?    [ yes / no ]
3. Single-threaded acceptable, or threads REQUIRED?  [ single-threaded OK / threads needed ]
4. True footprint number (bytes):                    ____
5. Required CSP directive set (+ any relaxations):    ____
6. crypto.subtle.digest: full-buffer or chunked?     [ full / chunked ]
7. Cache API vs OPFS recommendation:                 [ Cache API / OPFS ]
8. Two-pass vs single-pass call:                     [ two-pass / single-pass Zipformer-only ]

### >>> BRANCH DECISION <<<
- v1 ASR shape: [ two-pass base.en everywhere / two-pass + Moonshine mobile finalizer / single-pass Zipformer-only ]
- Threading/host: [ single-threaded static Pages / threads → COOP+COEP non-Pages host ]
- Storage: [ Cache API / OPFS ]   Hashing: [ full / chunked ]
- CSP posture: [ draft strict works / wider set: ____ / needs 'unsafe-eval' → REASSESS privacy ]

GO / NO-GO: ____
```

### How each answer maps to the plan's branches

- **base.en fast desktop + mobile, loads under CSP** → baseline: two-pass everywhere,
  single-threaded, static Pages, single-host CSP.
- **base.en fast desktop, slow mobile** → R27 fallback: Moonshine mobile finalizer (U6 conditional).
- **base.en slow on desktop too** → single-pass Zipformer-only v1 (drop the Whisper finalizer in U6).
- **single-threaded too slow → threads required** → COOP/COEP + a header shim or non-Pages host
  (e.g. Cloudflare Pages); **U10 CSP and U12 deploy target change** — a host migration, not a drop-in.
- **loader needs CSP beyond the expected relaxations** → document the wider set in U10; if it needs
  `'unsafe-eval'`, reassess the privacy posture before proceeding.
