<div align="center">

# chat-servant

**A copilot pinned to your own WeChat chat window — it reads the newest message, classifies the intent, drafts candidate replies, and fills them into the input box at the tier you chose.**

**默认只填入，不发送。截图不落盘，key 不进日志。**

[![License: BSL 1.1](https://img.shields.io/badge/license-BSL_1.1-orange.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-black.svg)](https://www.rust-lang.org/)
[![Windows](https://img.shields.io/badge/platform-Windows%2010%2B-0078D4.svg)](https://learn.microsoft.com/windows/)
[![CI](https://github.com/SeasonTemple/chat-servant/actions/workflows/ci.yml/badge.svg)](https://github.com/SeasonTemple/chat-servant/actions/workflows/ci.yml)
[![Release: v0.1.0](https://img.shields.io/badge/release-v0.1.0-brightgreen.svg)](https://github.com/SeasonTemple/chat-servant/releases)

[English](README.md) · [简体中文](README.zh-CN.md) · [Privacy](#privacy-boundaries) · [Getting started](#getting-started) · [Probes](#developer-probes) · [Gate](#the-gate) · [Contributing](CONTRIBUTING.md)

</div>

---

## Why it is shaped this way

Talking to a busy group chat at work is a context-switch problem, not a
reading problem. chat-servant sits next to the window you already have open,
does the reading, and hands you a draft.

Three decisions do all the work:

- **It does not understand the conversation. It observes and transcribes.** No
  database access, no hooking, no decryption. One capture path: window-level
  WGC frames plus local OCR.
- **Sending is tiered, and the lowest tier is the default.** The app fills the
  input box and stops. Whether anything actually leaves your machine is a
  question the Rust side answers, not the UI.
- **Silence costs nothing.** No new message from them means no model call. Ten
  quiet minutes is ten minutes of zero tokens.

## Privacy boundaries

These are product requirements, not implementation details.

- **You only read conversations you can already read**, on your own device.
  It does not act on anyone else's behalf.
- **No hooking, no injection.** It never reads the other party's database,
  never decrypts, never touches their process memory.
- **Frames never touch disk.** They live in memory — not written, not logged,
  not uploaded. The single exception is the explicit debug command
  `cargo xtask probe`, which writes to `target/probe/` and tells you it did.
- **It never touches money.** Transfer, red-packet and payment elements are out
  of scope, in the UI and in the drafting prompt.
- **Keys are read from Windows Credential Manager or the environment, nowhere
  else** — never from a config file, never from a command line, never into a
  log, never into the frontend bundle. Every outbound string is redacted.
- **Golden-test fixtures come from real conversations**, so they are not in this
  repository. The tests that use them **skip** when the fixture is absent.

## Three switches, each defaulting to the safe side

| Switch | Default | What it governs |
| --- | --- | --- |
| Sending tier | **fill in only** | Text lands in the input box at the cursor. Sending requires you to raise the tier yourself. |
| Images outbound | **off** | Whether an image region from them is cropped, encoded, and sent to a model. |
| Session identity gate | locked on first frame | The app only acts on the one locked conversation. Re-lock by hand if it grabs the wrong one. |

The tier and the image gate are **held in Rust** (`TierCell` / `ImagesOut`).
The UI can only request a change — "will this message actually go out" is never
settled by the interface.

## Getting started

**Windows 10 1803+ only** (Windows Graphics Capture). Windows 11 works better.

```powershell
# Frontend deps — the tauri CLI is already a devDependency
cd ui; pnpm install; cd ..

# Dev mode (HMR; the frontend is served from devUrl)
cd ui; pnpm tauri dev

# Installers (msi + nsis)
cd ui; pnpm tauri build
```

The app starts a resident observation loop that grabs a frame per second.

### It runs with no API key

Capture, OCR, anchoring and the candidate cards all work without one — the
drafts simply come from an explicitly labelled preset transport, and each one is
marked as such so it can never be mistaken for model output.

### Configuring a key

Keys are read from exactly two places, and nowhere else:

1. **Windows Credential Manager**, target `chat-servant/<provider>`;
2. environment variable `<PROVIDER>_API_KEY`.

Set it from the settings screen; it goes into the credential manager. "Test
connection" hits the read-only `/v1/models` — **zero token cost**.

Supported providers: `openai`, `anthropic`.

## Developer probes

When something misbehaves, probe it — do not guess. All read-only except
`probe frame`.

```powershell
cargo xtask caps            # does this machine support WGC at all
cargo xtask occluded        # is the window covered right now
cargo xtask probe frame     # grab a frame into target/probe/ and eyeball pixels
cargo xtask read            # grab and OCR immediately, print what it read
cargo xtask perceive --file <png>   # perception: per-line classification + merge
cargo xtask measure         # calibration: bubble fill, ink, contrast
cargo xtask e2e             # end-to-end (dry-run by default)
cargo xtask secret-scan     # any key left in the repository
```

**Run `caps` and `occluded` first** when "it doesn't do anything" — they rule
out most "is the app broken?" confusion. Failures name the actual cause
(occluded / minimized / not visible) instead of just "capture failed".

## The gate

Nine steps, all green, before a commit lands:

```powershell
pwsh -NoProfile -File scripts\gate.ps1
```

Encoding scan · `cargo fmt` · `cargo clippy -D warnings` · `cargo test` ·
secret-scan · `cargo deny check` · `svelte-check` · lint + svelte-autofixer ·
`vite build`. `.github/workflows/ci.yml` mirrors it step for step — and
`scripts/check-gate-parity.ps1` is what makes that claim true: it derives the
step list from `gate.ps1` and fails if `ci.yml` misses any of them.

> Read the `GATE: ALL GREEN` / `GATE: FAILED` line, not just the exit code.

## Architecture

| Crate | Role |
| --- | --- |
| `crates/core` | Domain types and pure logic. `#![forbid(unsafe_code)]` |
| `crates/capture` | Window discovery + Windows Graphics Capture + OCR |
| `crates/perceive` | Classifies OCR boxes into speakers, behind the session gate |
| `crates/reason` | **The only stage allowed to reach the network** |
| `xtask` | Developer probes |
| `ui/` | SvelteKit 5 frontend · `ui/src-tauri/` Tauri 2 shell |

Rust is the single source of truth: the frontend renders what the backend
sends and holds no second copy of session state.

## Docs

- [`CONTEXT.md`](CONTEXT.md) — domain terms and boundaries. **Read before coding.**
- [`docs/adr/`](docs/adr/) — decision records, including what was rejected
- [`docs/release-notes/`](docs/release-notes/) — curated bilingual release notes
- [`AGENTS.md`](AGENTS.md) — working agreements for agents
- [`.scratch/`](.scratch/) — tickets

## License

[Business Source License 1.1](LICENSE) © SeasonTemple — converts to
Apache-2.0 on **2029-10-12**.
