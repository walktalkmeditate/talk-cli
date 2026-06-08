# talk-cli

> A terminal listening companion — speak a reflection, and your words settle into stillness, right where you already work.

The third pillar of **Walk · Talk · Meditate**, in the terminal. Sibling to
[`meditate-cli`](../meditate-cli) (breathe) — this one is **talk** (reflect).

`meditate-cli` gave the terminal a breath. `talk-cli` gives it an ear.

```
$ meditate --for 5m        # breathe
  ...bell...
$ talk                     # the terminal asks, then listens

  "What are you carrying that isn't yours?"
  ● recording  ——▸▁▃▇▅▁   (on-device, no network)
  [space] done

  → ~/zettelkasten/2026-06-04-reflection.md
  "Stillness carries forward."
```

## What it is (and isn't)

- **Is:** voice reflection in the terminal. The system speaks a question, you
  answer aloud, it transcribes **on-device**, and the words land in a quiet file
  you can return to. Voice in, peace out.
- **Is not:** a Wispr Flow clone. Not system-wide dictation, not "type into any
  app," not a productivity tool. That corner is crowded and off-ethos. See
  [`DREAMING.md`](./DREAMING.md) for why we chose the path we did.

Two surfaces, one `talk-core`: a **terminal** binary, and a **fully-local web
page** (in-browser transcription, nothing touches a server) that may well be the
first thing we ship — see [`DREAMING.md` §9](./DREAMING.md#9-web-local-target-no-server-in-browser--feasible-maybe-v1).

## Status

🌱 **Dreaming.** No code yet. This repo currently holds the thinking that will
become a spec. Everything we've explored — the chosen direction, the question
framework, the "make it pretty" engine, the live-transcribe *settle*, and the
open-source engine choice — lives in **[`DREAMING.md`](./DREAMING.md)**.

## Inherited constraints (from the ecosystem)

- **MIT**, **zero-network**, **zero-telemetry** — like `meditate-cli`. No
  account, no API key, no phone-home.
- **Rust**, single binary, distributed via the `walktalkmeditate/tap` Homebrew
  tap + npm, with a `talk-core` / `talk-wasm` workspace mirroring
  `meditate-core` / `meditate-wasm`.
- Contemplative, minimalist voice. Slow and chill is the motto.

---

*Part of [momentmaker](https://github.com/momentmaker) · the
[walktalkmeditate](../walktalkmeditate) pilgrimage framework.*
