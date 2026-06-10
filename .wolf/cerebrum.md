# Cerebrum

> OpenWolf's learning memory. Updated automatically as the AI learns from interactions.
> Do not edit manually unless correcting an error.
> Last updated: 2026-06-10

## User Preferences

<!-- How the user likes things done. Code style, tools, patterns, communication. -->

## Key Learnings

- **Project:** talk
- **Description:** > A terminal listening companion — speak a reflection, and your words settle into stillness, right where you already work.
- **cleanup.rs has TWO filler sets, do not conflate:** `FILLERS` (um/uh/er/ah/like/you/know/so/well/i/mean) is ONLY for `guard_accepts`'s both-sides content-word comparison — it is a moat for an LLM rewriter, NOT a strip list. `deterministic_light`/`strip_leading_fillers` strips ONLY `LEADING_DISFLUENCIES` (non-lexical: um/uh/er/ah/mm/hmm/uhm/erm/hm). Stripping a leading real word (esp. "I"/"You") drops the user's meaning — restraint wins.
- **Transcript word changes have two possible origins — check the `<!-- raw: ... -->` comment to localize.** The stored raw is the STT text (pass-2 if a revise landed); the visible line is post-processed. If raw already shows the changed word, it's the MODEL (pass-2 Moonshine base, which can mishear a word the pass-1 zipformer got right); if raw differs from visible, it's post-processing. Pass-2 is usually better (recovers words pass-1 drops) but not per-word-guaranteed.

- **Resampler is now anti-aliased (src/listen/resample.rs, stateful windowed-sinc, HALF=32 Blackman).** The mic is 48kHz; the OLD naive-linear downsample had no low-pass and aliased >8kHz into the speech band. It holds HALF input-samples of right-context latency (the finish-flush/last chunk loses ~0.67ms at 48k — sub-phoneme, fine); reset() on pause purges pre-pause audio. Falls back to identity at exactly 16000. A kernel table for integer ratios was deliberately NOT added (YAGNI: ~1M transcendentals/s is orders below the ONNX bottleneck).

## Do-Not-Repeat

<!-- Mistakes made and corrected. Each entry prevents the same mistake recurring. -->
<!-- Format: [YYYY-MM-DD] Description of what went wrong and what to do instead. -->

- [2026-06-10] Do not use the `FILLERS` set as a leading-strip list in `deterministic_light` — it contains content words (i/you/so/well). It exists for the guard's comparison only. Leading-strip is `LEADING_DISFLUENCIES` only. (bug-007)

## Decision Log

<!-- Significant technical decisions with rationale. Why X was chosen over Y. -->

- [2026-06-10] Pass-2 model swapped from Moonshine base to Whisper base.en (int8). Whisper config uses `cfg.model_config.whisper.{encoder,decoder,language,task}` + `cfg.model_config.tokens`. Envelope widened from 8s to 30s. `suspect_hallucination` guard added to BOTH rescue branches (endpoint + flush-finish) only — not the normal pass-2 path.
