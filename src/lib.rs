//! SoundMixer core: per-app volume, routing and effects for macOS using Core Audio process taps.
//!
//! Built as a static library that the SwiftUI app in `mac/` links. Layers, bottom up:
//! [`audio`] (Core Audio, DSP, tap engine) → [`prefs`] (saved settings) → [`core`] (state and
//! commands) → [`ffi`] (the C interface Swift calls).

pub mod audio;
pub mod core;
pub mod ffi;
pub mod prefs;
