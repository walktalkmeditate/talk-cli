// =============================================================================
// talk-web U1 spike — instrumentation
//
// THROWAWAY. Lives only in web/spike/. Does NOT import from web/src/ and must
// never leak into the shipped app. This file is a *measurement scaffold*: the
// instrumentation is real; the sherpa-onnx ASR integration is a clearly-marked
// seam the human completes by dropping in the real Emscripten loader + models.
//
// What it measures (gates Phase C — see docs/plans/2026-06-13-001-feat-talk-web-plan.md §U1):
//   1. RTF (real-time factor) for the live edge + the Whisper base.en finalize
//   2. True cached footprint via navigator.storage.estimate()
//   3. crypto.subtle.digest cost: full-buffer vs chunked, on the ~199 MB file
//   4. CSP directives the sherpa-onnx loader violates (securitypolicyviolation)
//   5. crossOriginIsolated / SharedArrayBuffer / threads-requested / single-threaded
//   6. Module/asset shape: filenames + count fetched; ArrayBuffer vs Emscripten FS
//   7. Zero-egress sanity: every network Request after the "models loaded" mark
//
// HONESTY RULE: nothing here fabricates a measurement. Every value that depends
// on the (unwired) sherpa loader reports "seam not wired" until the human wires
// it. Search for `WIRE:` to find every integration seam.
// =============================================================================

// ---------------------------------------------------------------------------
// DOM helpers (typed, no framework)
// ---------------------------------------------------------------------------
const $ = <T extends HTMLElement = HTMLElement>(id: string): T => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`#${id} missing from index.html`);
  return el as T;
};

type ValState = "ok" | "warn" | "err" | "pending";
const setMetric = (id: string, text: string, state: ValState = "ok"): void => {
  const el = $(id);
  el.textContent = text;
  el.className = `val-${state}`;
};

type LogKind = "info" | "viol" | "net";
const logEl = $("log");
const logLines: string[] = [];
function log(kind: LogKind, msg: string): void {
  const stamp = new Date().toISOString().slice(11, 23);
  const line = `${stamp}  ${msg}`;
  logLines.push(`[${kind}] ${line}`);
  const row = document.createElement("div");
  row.className = `row ${kind}`;
  row.textContent = line;
  logEl.appendChild(row);
  logEl.scrollTop = logEl.scrollHeight;
}

// ---------------------------------------------------------------------------
// Collected results — fed into the go/no-go memo on "Copy results"
// ---------------------------------------------------------------------------
interface Results {
  rtfLive: string;
  rtfFinal: string;
  footprintBytes: number | null;
  footprintQuota: number | null;
  digestFullMs: number | null;
  digestChunkMs: number | null;
  digestFileBytes: number | null;
  cspViolations: Set<string>;
  crossOriginIsolated: boolean;
  sharedArrayBuffer: boolean;
  threadsRequested: "yes" | "no" | "unknown";
  ranSingleThreaded: "yes" | "no" | "unknown";
  moduleCount: number | null;
  moduleNames: Set<string>;
  assetShape: string;
}

const results: Results = {
  rtfLive: "seam not wired",
  rtfFinal: "seam not wired",
  footprintBytes: null,
  footprintQuota: null,
  digestFullMs: null,
  digestChunkMs: null,
  digestFileBytes: null,
  cspViolations: new Set(),
  crossOriginIsolated: false,
  sharedArrayBuffer: false,
  threadsRequested: "unknown",
  ranSingleThreaded: "unknown",
  moduleCount: null,
  moduleNames: new Set(),
  assetShape: "seam not wired",
};

// ===========================================================================
// METRIC 4 — CSP violations
// The page applies the draft strict CSP via <meta> in index.html. Every
// directive the sherpa-onnx Emscripten loader violates fires this event. The
// union of blockedURI/violatedDirective tells the human the *real* required set.
// ===========================================================================
document.addEventListener("securitypolicyviolation", (e: SecurityPolicyViolationEvent) => {
  const key = `${e.effectiveDirective || e.violatedDirective} ← ${e.blockedURI || "(inline)"}`;
  results.cspViolations.add(key);
  log("viol", `CSP violation: ${e.effectiveDirective} blocked ${e.blockedURI || "inline/eval"} (orig: ${e.originalPolicy.slice(0, 60)}…)`);
});

// ===========================================================================
// METRIC 5 — threading capabilities
// ===========================================================================
function reportThreadingCaps(): void {
  results.crossOriginIsolated = self.crossOriginIsolated === true;
  results.sharedArrayBuffer = typeof SharedArrayBuffer !== "undefined";

  setMetric("m-coi", results.crossOriginIsolated ? "true" : "false",
    results.crossOriginIsolated ? "ok" : "warn");
  setMetric("m-sab", results.sharedArrayBuffer ? "available" : "absent",
    results.sharedArrayBuffer ? "ok" : "warn");

  log("info", `crossOriginIsolated=${results.crossOriginIsolated} · SharedArrayBuffer=${results.sharedArrayBuffer}`);
  if (!results.crossOriginIsolated) {
    log("info", "Not cross-origin isolated (no COOP/COEP). Threads need SAB → single-threaded WASM is the static-Pages path. This is the assumption U1 tests.");
  }
}

// ===========================================================================
// METRIC 7 (precursor) — zero-egress network log
// Wrap fetch so every Request is logged. After "models loaded" is marked,
// requests are flagged as egress candidates (a mini precursor to U10's canary).
// Also feeds METRIC 6 (module/asset shape) by recording fetched filenames.
// ===========================================================================
let modelsLoadedAt: number | null = null;
const fetchedUrls = new Set<string>();

const realFetch = window.fetch.bind(window);
window.fetch = ((input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
  const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
  fetchedUrls.add(url);
  results.moduleNames.add(url.split("/").pop() || url);
  results.moduleCount = fetchedUrls.size;
  setMetric("m-modcount", String(results.moduleCount));

  const afterLoad = modelsLoadedAt !== null;
  if (afterLoad) {
    log("net", `EGRESS CANDIDATE (post-load) fetch → ${url}`);
  } else {
    log("net", `fetch → ${url}`);
  }
  return realFetch(input as RequestInfo, init);
}) as typeof window.fetch;

// Also log XHR opens — the Emscripten loader sometimes uses XHR, not fetch.
const realXhrOpen = XMLHttpRequest.prototype.open;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
XMLHttpRequest.prototype.open = function (this: XMLHttpRequest, method: string, url: string | URL, ...rest: any[]) {
  const u = typeof url === "string" ? url : url.href;
  fetchedUrls.add(u);
  results.moduleNames.add(u.split("/").pop() || u);
  results.moduleCount = fetchedUrls.size;
  setMetric("m-modcount", String(results.moduleCount));
  log(modelsLoadedAt !== null ? "net" : "info", `${modelsLoadedAt !== null ? "EGRESS CANDIDATE (post-load) " : ""}XHR ${method} → ${u}`);
  // @ts-expect-error variadic passthrough to the real signature
  return realXhrOpen.call(this, method, url, ...rest);
};

// ===========================================================================
// METRIC 2 — true cached footprint via navigator.storage.estimate()
// ===========================================================================
const ENVELOPE_BYTES = 330 * 1024 * 1024; // ~330 MB models, per the plan
const fmtBytes = (n: number): string => {
  if (n < 1024) return `${n} B`;
  const mb = n / (1024 * 1024);
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  return `${(mb / 1024).toFixed(2)} GB`;
};

async function measureFootprint(): Promise<void> {
  if (!navigator.storage?.estimate) {
    setMetric("m-storage", "storage.estimate() unsupported", "err");
    log("info", "navigator.storage.estimate() unsupported in this browser.");
    return;
  }
  const est = await navigator.storage.estimate();
  const usage = est.usage ?? 0;
  const quota = est.quota ?? 0;
  results.footprintBytes = usage;
  results.footprintQuota = quota;

  const withinEnvelope = usage > 0 && usage <= ENVELOPE_BYTES * 1.15;
  const note = usage === 0
    ? "0 — load the models first, then re-measure"
    : `${fmtBytes(usage)} used / ${fmtBytes(quota)} quota (envelope ~330 MB)`;
  setMetric("m-storage", note, usage === 0 ? "warn" : withinEnvelope ? "ok" : "warn");
  log("info", `storage.estimate(): usage=${usage} (${fmtBytes(usage)}), quota=${quota} (${fmtBytes(quota)})`);

  // Persisted-storage opt-in is what protects the ~330 MB cache from eviction
  // (esp. iOS Safari). Report whether persistence is granted.
  if (navigator.storage.persisted) {
    const persisted = await navigator.storage.persisted();
    log("info", `storage.persisted()=${persisted} (U5 will call persist() opt-in)`);
  }
}

// ===========================================================================
// METRIC 3 — crypto.subtle.digest cost: full-buffer vs chunked
// The human picks the largest (~199 MB) model file. We time a single
// digest(whole ArrayBuffer) vs a manual chunked walk to learn whether mobile
// needs streaming/chunked hashing (U5's integrity.ts decision).
//
// NOTE on chunked hashing: Web Crypto's SubtleCrypto.digest has no incremental
// API, so a true streaming SHA-256 needs a JS/WASM implementation. Here we
// measure the *I/O + buffering* shape of a chunked read (the part that actually
// stresses mobile memory) and a full-buffer digest, so the human sees whether
// holding 199 MB in one ArrayBuffer is the bottleneck. The chunked number is the
// time to read the file in slices and feed a placeholder hasher; replace the
// placeholder with the chosen streaming-SHA-256 lib to get the real chunked cost.
// ===========================================================================
const CHUNK = 8 * 1024 * 1024; // 8 MB slices

async function timeDigest(file: File): Promise<void> {
  results.digestFileBytes = file.size;
  log("info", `digest target: ${file.name} (${fmtBytes(file.size)})`);

  // Full-buffer: read the whole file into one ArrayBuffer, then digest once.
  try {
    const t0 = performance.now();
    const buf = await file.arrayBuffer();
    const t1 = performance.now();
    await crypto.subtle.digest("SHA-256", buf);
    const t2 = performance.now();
    results.digestFullMs = t2 - t0;
    setMetric("m-digest-full",
      `${(t2 - t0).toFixed(0)} ms (read ${(t1 - t0).toFixed(0)} + hash ${(t2 - t1).toFixed(0)})`, "ok");
    log("info", `digest(full): total ${(t2 - t0).toFixed(0)} ms · read ${(t1 - t0).toFixed(0)} ms · hash ${(t2 - t1).toFixed(0)} ms`);
  } catch (err) {
    setMetric("m-digest-full", `failed (likely OOM): ${(err as Error).message}`, "err");
    log("info", `digest(full) FAILED — ${(err as Error).message}. This is the signal that mobile needs chunked hashing.`);
  }

  // Chunked: slice the file, never holding the whole thing. This measures the
  // memory-friendly path's I/O cost. WIRE: feed each chunk to a streaming
  // SHA-256 hasher (e.g. hash-wasm) to get the true chunked digest cost.
  try {
    const t0 = performance.now();
    let offset = 0;
    let bytesWalked = 0;
    while (offset < file.size) {
      const slice = file.slice(offset, Math.min(offset + CHUNK, file.size));
      const chunk = await slice.arrayBuffer();
      bytesWalked += chunk.byteLength;
      // WIRE: streamingHasher.update(new Uint8Array(chunk));
      // Placeholder work so the measurement isn't trivially optimized away:
      await crypto.subtle.digest("SHA-256", chunk);
      offset += CHUNK;
    }
    const t1 = performance.now();
    results.digestChunkMs = t1 - t0;
    setMetric("m-digest-chunk",
      `${(t1 - t0).toFixed(0)} ms over ${fmtBytes(bytesWalked)} in ${CHUNK / (1024 * 1024)} MB slices (per-chunk hash placeholder — wire a streaming SHA-256 for the true value)`, "ok");
    log("info", `digest(chunked): ${(t1 - t0).toFixed(0)} ms in ${CHUNK / (1024 * 1024)} MB slices (placeholder hasher)`);
  } catch (err) {
    setMetric("m-digest-chunk", `failed: ${(err as Error).message}`, "err");
    log("info", `digest(chunked) FAILED — ${(err as Error).message}`);
  }
}

// ===========================================================================
// METRIC 1 — RTF (real-time factor), wired to the sherpa recognizer
//
// RTF = wall-clock time spent recognizing ÷ duration of the audio recognized.
// RTF < 1.0 means faster-than-real-time. The helper below is what the sherpa
// callbacks call; the recognizer itself is the seam.
// ===========================================================================
class RtfMeter {
  private audioMs = 0;
  private computeMs = 0;
  constructor(private readonly metricId: string, private readonly resultKey: "rtfLive" | "rtfFinal") {}

  record(audioDurationMs: number, computeDurationMs: number): void {
    this.audioMs += audioDurationMs;
    this.computeMs += computeDurationMs;
    const rtf = this.audioMs > 0 ? this.computeMs / this.audioMs : 0;
    const text = `${rtf.toFixed(3)} (cum ${(this.computeMs).toFixed(0)} ms compute / ${(this.audioMs / 1000).toFixed(1)} s audio)`;
    results[this.resultKey] = text;
    setMetric(this.metricId, text, rtf < 0.6 ? "ok" : rtf < 1.0 ? "warn" : "err");
  }
}
const liveRtf = new RtfMeter("m-rtf-live", "rtfLive");
const finalRtf = new RtfMeter("m-rtf-final", "rtfFinal");

// ===========================================================================
// Mic capture → 16 kHz mono frames → recognizer
// AudioWorklet preferred; ScriptProcessor fallback for older Safari.
// ===========================================================================
const TARGET_SR = 16000;
let audioCtx: AudioContext | null = null;
let mediaStream: MediaStream | null = null;
let workletNode: AudioWorkletNode | null = null;
let scriptNode: ScriptProcessorNode | null = null;
const capturedFrames: Float32Array[] = []; // for the "Finalize now" pass

const partialsEl = $("partials");
const finalEl = $("final");

async function startMic(): Promise<void> {
  capturedFrames.length = 0;
  audioCtx = new AudioContext({ sampleRate: TARGET_SR });
  if (audioCtx.sampleRate !== TARGET_SR) {
    log("info", `Requested ${TARGET_SR} Hz; got ${audioCtx.sampleRate} Hz — frames need resampling before the recognizer. WIRE: resample to 16 kHz (mirror src/listen/resample.rs).`);
  }

  mediaStream = await navigator.mediaDevices.getUserMedia({
    audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    video: false,
  });
  const source = audioCtx.createMediaStreamSource(mediaStream);

  if (audioCtx.audioWorklet) {
    await audioCtx.audioWorklet.addModule("./mic-worklet.js");
    workletNode = new AudioWorkletNode(audioCtx, "mic-capture");
    workletNode.port.onmessage = (e: MessageEvent<Float32Array>) => onFrame(e.data);
    source.connect(workletNode);
    // The worklet has no audible output; connect to destination is unnecessary.
    log("info", "Mic started via AudioWorklet at 16 kHz mono.");
  } else {
    // ScriptProcessor fallback (deprecated, but iOS Safari < 14.5 lacks worklet).
    scriptNode = audioCtx.createScriptProcessor(4096, 1, 1);
    scriptNode.onaudioprocess = (e) => onFrame(e.inputBuffer.getChannelData(0).slice(0));
    source.connect(scriptNode);
    scriptNode.connect(audioCtx.destination);
    log("info", "Mic started via ScriptProcessor fallback (no AudioWorklet) at 16 kHz mono.");
  }

  $<HTMLButtonElement>("btn-start").disabled = true;
  $<HTMLButtonElement>("btn-stop").disabled = false;
  $<HTMLButtonElement>("btn-finalize").disabled = false;
}

function onFrame(frame: Float32Array): void {
  capturedFrames.push(frame);
  const frameMs = (frame.length / TARGET_SR) * 1000;

  // ----------------------------------------------------------------------
  // WIRE: wire to sherpa-onnx WASM streaming Zipformer recognizer here.
  //
  //   const t0 = performance.now();
  //   streamingRecognizer.acceptWaveform(TARGET_SR, frame);
  //   while (streamingRecognizer.isReady()) streamingRecognizer.decode();
  //   const partial = streamingRecognizer.getResult().text;
  //   const t1 = performance.now();
  //   liveRtf.record(frameMs, t1 - t0);   // METRIC 1 — live edge RTF
  //   partialsEl.textContent = partial;
  //
  // Reference loader to copy (streaming Zipformer EN):
  //   https://huggingface.co/spaces/k2-fsa/web-assembly-asr-sherpa-onnx-en
  // VAD + Moonshine (mobile-fallback comparison):
  //   https://huggingface.co/spaces/k2-fsa/web-assembly-vad-asr-sherpa-onnx-moonshine
  // ----------------------------------------------------------------------
  void frameMs; // referenced once wired
}

async function stopMic(): Promise<void> {
  workletNode?.disconnect();
  scriptNode?.disconnect();
  mediaStream?.getTracks().forEach((t) => t.stop());
  await audioCtx?.close();
  workletNode = null;
  scriptNode = null;
  mediaStream = null;
  audioCtx = null;
  $<HTMLButtonElement>("btn-start").disabled = false;
  $<HTMLButtonElement>("btn-stop").disabled = true;
  $<HTMLButtonElement>("btn-finalize").disabled = true;
  log("info", `Mic stopped. ${capturedFrames.length} frames captured.`);
}

function finalizeNow(): void {
  const totalSamples = capturedFrames.reduce((n, f) => n + f.length, 0);
  const audioMs = (totalSamples / TARGET_SR) * 1000;
  log("info", `Finalize requested over ${(audioMs / 1000).toFixed(1)} s of buffered audio.`);

  // ----------------------------------------------------------------------
  // WIRE: wire to sherpa-onnx WASM Whisper base.en (offline) recognizer here.
  // On mobile, this is where the U1 decision lands base.en vs the Moonshine
  // tiny finalizer — run both builds and compare the RTF readout.
  //
  //   const pcm = concatFrames(capturedFrames);
  //   const t0 = performance.now();
  //   const stream = offlineRecognizer.createStream();
  //   stream.acceptWaveform(TARGET_SR, pcm);
  //   offlineRecognizer.decode(stream);
  //   const text = offlineRecognizer.getResult(stream).text;
  //   const t1 = performance.now();
  //   finalRtf.record(audioMs, t1 - t0);   // METRIC 1 — finalize RTF
  //   finalEl.textContent = text;
  //
  //   results.assetShape = "consumes ArrayBuffer" | "stages into Emscripten FS";
  //   results.threadsRequested = sherpaConfig.numThreads > 1 ? "yes" : "no";
  //   results.ranSingleThreaded = (!self.crossOriginIsolated) ? "yes" : "...";
  //   setMetric("m-threads", ...); setMetric("m-singlethread", ...);
  //   setMetric("m-assetshape", results.assetShape);
  // ----------------------------------------------------------------------
  void audioMs;
  void finalRtf;
}

// ===========================================================================
// CSP relax toggle — re-inject a permissive policy to confirm the loader works
// at all, then tighten. See the caveat note in index.html (#csp-note).
// ===========================================================================
function applyPermissiveCsp(on: boolean): void {
  const existing = document.getElementById("csp-permissive");
  if (on && !existing) {
    const meta = document.createElement("meta");
    meta.id = "csp-permissive";
    meta.httpEquiv = "Content-Security-Policy";
    // Deliberately wide — for the "does the loader run at all" confirmation only.
    meta.content =
      "default-src * 'unsafe-inline' 'unsafe-eval' data: blob:; " +
      "script-src * 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' blob:; " +
      "worker-src * blob:; connect-src *;";
    document.head.appendChild(meta);
    log("info", "Permissive CSP meta injected. CAVEAT: browsers INTERSECT multiple CSPs — a strict meta already applied at parse time CANNOT be loosened. If the loader still fails, edit the strict meta in index.html (or serve a permissive header), reload, then tighten directive-by-directive.");
  } else if (!on && existing) {
    existing.remove();
    log("info", "Permissive CSP meta removed (note: removal does not restore blocking if the strict policy was already in force at load).");
  }
}

// ===========================================================================
// METRIC 6 reporting helper — call once the loader has fetched its assets.
// ===========================================================================
function reportModuleShape(): void {
  if (results.moduleCount === null) {
    log("info", "No fetches recorded yet — load the recognizer first, then the module count populates.");
    return;
  }
  log("info", `Modules/assets fetched (${results.moduleCount}): ${[...results.moduleNames].join(", ")}`);
}

// ===========================================================================
// Go/no-go memo — copied to clipboard, structured to feed the plan's decision tree
// ===========================================================================
function buildMemo(): string {
  const cspMeta = document.getElementById("csp-strict") as HTMLMetaElement | null;
  const ua = navigator.userAgent;
  const violations = results.cspViolations.size
    ? [...results.cspViolations].map((v) => `    - ${v}`).join("\n")
    : "    - (none caught — either the loader isn't wired yet, or it loads cleanly under the draft strict CSP)";
  const modNames = results.moduleNames.size ? [...results.moduleNames].join(", ") : "(none fetched yet)";
  const footprint = results.footprintBytes
    ? `${fmtBytes(results.footprintBytes)} (${results.footprintBytes} bytes)`
    : "(not measured — load models, then 'Measure footprint')";

  return `## U1 spike go/no-go memo  (paste into the plan's Open Questions)

> Generated by web/spike on ${new Date().toISOString()}
> Device/UA: ${ua}
> Draft strict CSP applied: ${cspMeta?.content ?? "(meta missing!)"}
> THROWAWAY harness — fill the [HUMAN] lines from your two-device runs.

### Raw measurements (auto-captured this session)
- RTF live edge (streaming Zipformer): ${results.rtfLive}
- RTF finalize (Whisper base.en / Moonshine): ${results.rtfFinal}
- Cached footprint (navigator.storage.estimate): ${footprint}
  - envelope check: ~330 MB expected
- crypto.subtle.digest full-buffer: ${results.digestFullMs !== null ? results.digestFullMs.toFixed(0) + " ms" : "(not run)"} over ${results.digestFileBytes ? fmtBytes(results.digestFileBytes) : "(no file picked)"}
- crypto.subtle.digest chunked (placeholder hasher): ${results.digestChunkMs !== null ? results.digestChunkMs.toFixed(0) + " ms" : "(not run)"}
- crossOriginIsolated: ${results.crossOriginIsolated}
- SharedArrayBuffer available: ${results.sharedArrayBuffer}
- threads requested by sherpa build: ${results.threadsRequested}
- ran single-threaded: ${results.ranSingleThreaded}
- modules/assets fetched (${results.moduleCount ?? 0}): ${modNames}
- asset consumption shape: ${results.assetShape}
- CSP violations caught:
${violations}

### Decision-tree answers  [HUMAN — fill from desktop + mid-range phone runs]
1. base.en fast enough on DESKTOP single-threaded?           [ yes / no ]  RTF: ____
2. base.en fast enough on MID-RANGE PHONE?                    [ yes / no ]  RTF: ____
   - if no: does Moonshine tiny meet the bar on phone?        [ yes / no ]  RTF: ____
3. Single-threaded acceptable, or are threads REQUIRED?       [ single-threaded OK / threads needed ]
4. True cached footprint number (bytes):                      ____  (matches ~330 MB? __)
5. Required CSP directive set (strict draft + any relaxations the loader forced):
   default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob'; connect-src 'self' https://cdn.pilgrimapp.org;
   + observed extra relaxations: ____
6. crypto.subtle.digest: full-buffer OK on mobile, or chunked/streaming REQUIRED? [ full / chunked ]
7. Cache API vs OPFS recommendation (by measured write/footprint behavior):       [ Cache API / OPFS ]
8. Zero-egress sanity: any post-"models loaded" requests beyond connect-src?      [ clean / suspicious: ____ ]

### >>> BRANCH DECISION (the load-bearing output) <<<
- v1 ASR shape:     [ two-pass base.en everywhere / two-pass + Moonshine mobile finalizer / single-pass Zipformer-only ]
- Threading/host:   [ single-threaded static Pages / threads → COOP+COEP non-Pages host (Cloudflare Pages) ]
- Storage:          [ Cache API / OPFS ]
- Hashing:          [ full-buffer digest / chunked streaming SHA-256 ]
- CSP posture:      [ draft strict set works / wider set needed: ____ / requires 'unsafe-eval' → REASSESS privacy posture ]

GO / NO-GO: ________________
`;
}

async function copyMemo(): Promise<void> {
  const memo = buildMemo();
  try {
    await navigator.clipboard.writeText(memo);
    log("info", "Go/no-go memo copied to clipboard. Paste it into the plan's Open Questions and fill the [HUMAN] lines.");
  } catch {
    // Clipboard API may be blocked (insecure context / permissions). Fall back.
    log("info", "Clipboard write blocked — memo dumped to the console (copy from there).");
    // eslint-disable-next-line no-console
    console.log(memo);
  }
}

// ===========================================================================
// Wire up the UI
// ===========================================================================
function wireUi(): void {
  $<HTMLButtonElement>("btn-start").addEventListener("click", () =>
    startMic().catch((e) => log("info", `startMic failed: ${(e as Error).message}`)));
  $<HTMLButtonElement>("btn-stop").addEventListener("click", () =>
    stopMic().catch((e) => log("info", `stopMic failed: ${(e as Error).message}`)));
  $<HTMLButtonElement>("btn-finalize").addEventListener("click", finalizeNow);
  $<HTMLButtonElement>("btn-storage").addEventListener("click", () =>
    measureFootprint().catch((e) => log("info", `footprint failed: ${(e as Error).message}`)));

  $<HTMLButtonElement>("btn-digest").addEventListener("click", () => {
    const picker = document.createElement("input");
    picker.type = "file";
    picker.onchange = () => {
      const f = picker.files?.[0];
      if (f) timeDigest(f).catch((e) => log("info", `digest failed: ${(e as Error).message}`));
    };
    picker.click();
  });

  $<HTMLInputElement>("toggle-csp").addEventListener("change", (e) =>
    applyPermissiveCsp((e.target as HTMLInputElement).checked));

  $<HTMLButtonElement>("btn-mark-loaded").addEventListener("click", () => {
    modelsLoadedAt = performance.now();
    reportModuleShape();
    log("info", "=== MODELS LOADED marker set. Any fetch/XHR after this is an egress candidate (U10 canary precursor). ===");
  });
  $<HTMLButtonElement>("btn-clear-log").addEventListener("click", () => {
    logEl.replaceChildren();
    logLines.length = 0;
  });
  $<HTMLButtonElement>("btn-copy").addEventListener("click", () => void copyMemo());
}

// ===========================================================================
// Boot
// ===========================================================================
function boot(): void {
  wireUi();
  reportThreadingCaps();
  measureFootprint().catch(() => undefined); // baseline (pre-load) footprint
  log("info", "U1 spike harness ready. Draft strict CSP applied via <meta>. Wire the sherpa-onnx loader at the `WIRE:` seams, then run on desktop + a mid-range phone.");
  log("info", "Recognizer NOT wired — RTF/partials/final stay empty until you complete the seams. No measurements are fabricated.");
}

boot();
