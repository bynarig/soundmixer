# AGENTS.md

Context for coding agents working on SoundMixer. For people using the app, see `README.md`.

## What this is

A macOS app that sets the volume, output device and effects for each app separately, using Core Audio process taps (macOS 14.2+). It has two halves:

- **Rust core** (repo root, crate `soundmixer`, built as a static library): the audio engine, effects, saved settings and all the logic.
- **SwiftUI app** (`mac/`): the window. It uses only standard SwiftUI controls, which macOS 26+ draws in Liquid Glass automatically, following Apple's HIG. Do not add custom glass inside the content.


## Commands

```sh
scripts/check.sh                # everything below, warnings as errors (what hooks and CI run)

# Rust core (repo root): every one of these must produce zero warnings
cargo fmt                       # rustfmt.toml: max_width 140, heuristics "Max"
cargo clippy --all-targets
cargo test                      # headless, never touches audio hardware or saved settings
cargo doc --no-deps --document-private-items

# SwiftUI app
cd mac && swift build && swift test          # links ../target/release/libsoundmixer.a: run `cargo build --release` first
mac/build-app.sh                             # → mac/build/SoundMixer.app (ad-hoc signed)
open -n mac/build/SoundMixer.app --args --preview    # see the UI with audio OFF
```

- **`--preview`** creates no taps, never changes devices and never saves settings. Use it whenever you want to look at the UI. To capture the window, run `screencapture -x -o -l <windowid>`; get the window id from `CGWindowListCopyWindowInfo` for the app's pid.
- **Never launch the app without `--preview` unless the user asks.** A normal launch applies the user's saved taps to their real audio.

## Layout

```
Cargo.toml, rustfmt.toml
src/
  lib.rs                 crate root (staticlib + rlib)
  ffi.rs                 C interface: sm_new / sm_tick / sm_command / sm_free / sm_string_free (JSON strings)
  core/
    mod.rs               Core state, Command (what the UI sends), Snapshot (what the UI shows)
    sync.rs              polling Core Audio, grouping processes into apps, building tap specs, tick
    tests.rs
  prefs.rs               saved settings: ~/Library/Application Support/SoundMixer/settings.json
  audio/
    ca.rs                thin wrappers over the Core Audio HAL (adapter; HAL FFI lives here)
    dsp.rs               allocation-free effect chain: EQ, parametric EQ, boost, smoother, limiter
    engine.rs            real-time tap engine: one tap + private aggregate device per adjusted app
mac/
  Package.swift          SwiftPM: CSoundMixer (C header + modulemap) → SoundMixer app → tests
  Info.plist             bundle id, NSAudioCaptureUsageDescription, minimum macOS
  build-app.sh
  Sources/CSoundMixer/soundmixer.h       must match src/ffi.rs
  Sources/SoundMixer/
    SoundMixerApp.swift  @main, Window + Settings scenes, quits cleanly (sm_free)
    Mixer.swift          @Observable bridge: polls sm_tick at 20 Hz, sends commands
    Model.swift          Codable mirrors of Snapshot / AppPrefs / DevicePrefs (snake_case JSON)
    Scale.swift          pure mappings: volume ↔ slider position, meter dB, labels
    ContentView.swift    grouped Form, toolbar, alerts, Settings, shared rows (DevicePicker, VolumeRow)
    DeviceSections.swift Output / Input sections, headphone EQ file import, BalanceSlider
    AppsSection.swift    app rows (icon, meter, volume), AppOptions popover
    Equalizer.swift      EqualizerRow + popover with 10 band sliders and presets
    Updater.swift        self-updater: signed appcast.json from the latest GitHub release
    UpdatesSection.swift Settings → Updates
  Updates.json           update feed URL + Ed25519 public key (baked into Info.plist by build-app.sh)
  Tests/SoundMixerTests/
scripts/check.sh            the full check (Rust + Swift + app bundle); run by git hooks and CI
scripts/setup-hooks.sh      enables .githooks (pre-commit tests the staged snapshot, pre-push the tree)
scripts/new_release.sh      bump version, commit, tag, push → release.yml builds, signs and publishes
scripts/release.sh          universal build + Ed25519-signed zip + appcast.json (run by release.yml)
scripts/update_tool.swift   Ed25519 keys and signatures (CryptoKit)
scripts/setup_codesign.sh   once: permanent self-signed "SoundMixer Code Signing" identity
scripts/setup_ci_secrets.sh once: upload the signing keys as GitHub Actions secrets
scripts/make_icon.swift     renders the app icon
.github/workflows/          ci.yml (every push/PR, reused by release), release.yml (tag vX.Y.Z)
docs/                       GitHub Pages site (https://bynarig.github.io/soundmixer/)
```

Dependencies only point one way: SwiftUI → `ffi` → `core` → `prefs` → `audio`.

## Patterns in use (keep them)

- **Thin UI, smart core.** Swift only shows the `Snapshot` and sends `Command`s. Logic, validation and persistence stay in Rust. To add a feature:
  1. Add a field to `Snapshot` and/or a variant to `Command` (`src/core/mod.rs`), with a test in `core/tests.rs`.
  2. Mirror the change in `Model.swift` and `Mixer.swift`.
  3. Draw it.
- **JSON across the boundary.** Rust keys are snake_case, and Swift converts them with `JSONDecoder.core` / `JSONEncoder.core`. `AppPrefs`, `DevicePrefs` and `PeqSettings.bands` (always 16 entries) must round-trip exactly. Every FFI function catches panics.
- **Adapter.** `audio::ca` is the only place that talks to the HAL.
- **Desired-state reconciliation.** `build_specs` lists the taps that should exist, and `Engine::reconcile` creates, retargets or crossfades them so audio is never cut abruptly. Set `need_reconcile = true` after any change that affects routing or which apps are tapped.
- **Lock-free parameter passing.** `SharedChain` carries slider values to the audio thread. The callback must stay allocation-free and lock-free.
- **Self-repairing settings.** `sanitize()` runs on load, on save, on every `SetApp`/`SetDevice`, and before values are shown. Treat disk contents, pasted profiles and UI input as untrusted.
- **Preview safety.** With no engine (`Core::new(false)`), the core never saves and never calls setters on the hardware. Keep it that way so tests and `--preview` are harmless.

## Rules for changes

- **Zero warnings everywhere:** `cargo build`, `cargo clippy --all-targets`, `cargo doc`, `swift build` and `swift test`. Every `#[allow]` needs a reason; the existing ones are `non_upper_case_globals` for FFI names, a `deprecated` HAL call, and `unused_unsafe` around objc2 calls.
- **HIG:**
  - Use standard controls in a grouped `Form` (the System Settings look).
  - Use switches for on/off, popovers for secondary settings, and the Settings window (⌘,) for preferences.
  - Use `.confirmationDialog` for destructive actions, and SF Symbols for icons.
  - Never use hard-coded colours except semantic ones (orange = boost above 100 %, red = muted or error).
- **Accessibility:** icon-only buttons use `Label` with `.labelStyle(.iconOnly)` so VoiceOver gets a name. Sliders get `.accessibilityLabel` and `.accessibilityValue`.
- **Tests and visual check:** new logic needs a test (Rust in `core/tests.rs` or the module, Swift in `mac/Tests`). Check UI changes with `--preview` in light and dark.
- **DSP constants:** they are documented next to their definitions in `dsp.rs` and `engine.rs`.
- **Commits:** run `scripts/setup-hooks.sh` once per clone. Never use `--no-verify`; fix what `scripts/check.sh` reports.
- **Releases:** `scripts/new_release.sh X.Y.Z "Short summary"`. `Cargo.toml` is the only version source; build-app.sh writes it into the bundle. The tag message becomes the release notes, and its first line is what the in-app update offer shows.
- **Updates:** the app trusts only zips signed with the private key in `~/.config/soundmixer/update_signing_key`, which matches `publicKey` in `mac/Updates.json`. Never change that key; existing installs would stop updating. Release builds must be signed with the "SoundMixer Code Signing" identity so macOS keeps the audio permission across updates.
