//! The app's brain, without any UI: it owns the saved settings and the audio engine, takes
//! commands from the interface and hands back a snapshot of everything the interface shows.
//!
//! The interface (SwiftUI, see `mac/`) talks to it through `crate::ffi` in JSON.

mod sync;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::audio::ca::{self, ObjId, Role};
use crate::audio::dsp;
use crate::audio::engine::{self, Engine, LatencyMode, SharedChain};
use crate::prefs::{AppPrefs, DevicePrefs, Prefs, MAX_APP_VOLUME};

/// How long an app stays listed after it stops playing.
const RECENT: Duration = Duration::from_secs(45);

#[derive(Clone)]
struct AppEntry {
    key: String,
    name: String,
    objects: Vec<ObjId>,
    playing: bool,
    last_playing: Option<Instant>,
    /// a normal Dock app (not a background helper or extension)
    regular: bool,
}

pub struct Core {
    prefs: Prefs,
    prefs_changed: Option<Instant>,
    engine: Option<Engine>,
    devices: Vec<ca::Device>,
    default_out: ObjId,
    default_in: ObjId,
    default_fx: ObjId,
    out_volume: Option<f32>,
    out_muted: Option<bool>,
    out_balance: Option<f32>,
    in_volume: Option<f32>,
    in_muted: Option<bool>,
    apps: Vec<AppEntry>,
    others: Vec<AppEntry>,
    ident_cache: HashMap<i32, (String, String, bool)>,
    app_chains: HashMap<String, Arc<SharedChain>>,
    dev_chains: HashMap<String, Arc<SharedChain>>,
    last_refresh: Option<Instant>,
    last_volumes: Option<Instant>,
    need_reconcile: bool,
    own_pid: i32,
    global_error: Option<String>,
    shown_meters: HashMap<String, (f32, f32)>,
}

// ------------------------------------------------------------------ what the UI sends

/// One change made by the person. Sent as JSON, e.g. `{"cmd":"set_latency","mode":"Low"}`.
#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    SetApp {
        key: String,
        prefs: Box<AppPrefs>,
    },
    SetDevice {
        uid: String,
        prefs: Box<DevicePrefs>,
    },
    /// Parse a pasted headphone profile (AutoEQ text or JSON) for an output device.
    LoadProfile {
        uid: String,
        text: String,
    },
    SetDefaultDevice {
        role: Role,
        id: ObjId,
    },
    SetDeviceVolume {
        input: bool,
        volume: f32,
    },
    SetDeviceMuted {
        input: bool,
        muted: bool,
    },
    SetOutputBalance {
        balance: f32,
    },
    SetLatency {
        mode: LatencyMode,
    },
    SetShowOtherApps {
        show: bool,
    },
    /// Every app back to 100 % with no effects (favourites stay favourites).
    ResetAll,
}

// ------------------------------------------------------------------ what the UI shows

#[derive(Debug, Serialize)]
pub struct Snapshot {
    /// No audio engine (the UI is being previewed); nothing is tapped.
    pub preview: bool,
    pub status: String,
    pub outputs: Vec<DeviceInfo>,
    pub inputs: Vec<DeviceInfo>,
    pub default_output: ObjId,
    pub default_input: ObjId,
    pub default_effects: ObjId,
    pub output: Option<OutputState>,
    pub input: InputState,
    pub apps: Vec<AppState>,
    pub others: Vec<AppState>,
    pub latency: LatencyMode,
    pub latency_options: Vec<LatencyOption>,
    pub show_other_apps: bool,
    pub max_app_volume: f32,
    /// Problems not tied to one app.
    pub messages: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceInfo {
    pub id: ObjId,
    pub uid: String,
    pub name: String,
    /// "Built-in", "USB", "Bluetooth", "AirPlay", …
    pub transport: &'static str,
}

#[derive(Debug, Serialize)]
pub struct OutputState {
    pub uid: String,
    pub volume: Option<f32>,
    pub muted: Option<bool>,
    pub balance: Option<f32>,
    /// e.g. "48 kHz · 2 channels · Built-in"
    pub detail: String,
    pub effects: DevicePrefs,
}

#[derive(Debug, Serialize)]
pub struct InputState {
    pub volume: Option<f32>,
    pub muted: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct AppState {
    pub key: String,
    pub name: String,
    /// Bundle identifier when known (for the app icon).
    pub bundle_id: Option<String>,
    pub playing: bool,
    /// Has audio objects right now (false: a favourite that is not running).
    pub running: bool,
    pub prefs: AppPrefs,
    /// Peak level per channel, 0..1, with meter ballistics applied.
    pub meter: [f32; 2],
    /// Audio goes through SoundMixer (a tap is running).
    pub tapped: bool,
    pub detail: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LatencyOption {
    pub mode: LatencyMode,
    pub label: &'static str,
}

const LATENCY_MODES: [LatencyMode; 4] = [LatencyMode::Auto, LatencyMode::Low, LatencyMode::Balanced, LatencyMode::Safe];

impl Core {
    /// `audio: false` never creates taps or changes devices (UI preview and tests).
    pub fn new(audio: bool) -> Self {
        let mut core = Self::with_prefs(Prefs::load(), audio.then(Engine::new));
        core.refresh(true);
        core
    }

    fn with_prefs(prefs: Prefs, engine: Option<Engine>) -> Self {
        Self {
            prefs,
            prefs_changed: None,
            engine,
            devices: Vec::new(),
            default_out: 0,
            default_in: 0,
            default_fx: 0,
            out_volume: None,
            out_muted: None,
            out_balance: None,
            in_volume: None,
            in_muted: None,
            apps: Vec::new(),
            others: Vec::new(),
            ident_cache: HashMap::new(),
            app_chains: HashMap::new(),
            dev_chains: HashMap::new(),
            last_refresh: None,
            last_volumes: None,
            need_reconcile: true,
            own_pid: std::process::id() as i32,
            global_error: None,
            shown_meters: HashMap::new(),
        }
    }

    /// Apply one change. Ok carries an optional message for the person, Err a problem.
    pub fn apply(&mut self, cmd: Command) -> Result<Option<String>, String> {
        let audio = self.engine.is_some();
        match cmd {
            Command::SetApp { key, prefs } => {
                let mut prefs = *prefs;
                prefs.sanitize();
                let before = self.prefs.apps.get(&key).cloned().unwrap_or_default();
                if prefs.favorite != before.favorite || prefs.output != before.output || prefs.effects_neutral() != before.effects_neutral()
                {
                    self.need_reconcile = true;
                }
                if prefs != before {
                    self.prefs.apps.insert(key, prefs);
                    self.prefs_changed = Some(Instant::now());
                }
            }
            Command::SetDevice { uid, prefs } => {
                let mut prefs = *prefs;
                prefs.sanitize();
                self.prefs.devices.insert(uid, prefs);
                self.mark_prefs();
            }
            Command::LoadProfile { uid, text } => {
                let (name, peq) = dsp::parse_profile(&text)?;
                let d = self.prefs.devices.entry(uid).or_default();
                d.peq = peq;
                d.peq.enabled = true;
                d.peq_name = Some(name);
                d.sanitize();
                let n = d.peq.count;
                self.mark_prefs();
                return Ok(Some(format!("Loaded {n} filters")));
            }
            Command::SetDefaultDevice { role, id } => {
                if audio && ca::set_default_device(role, id) {
                    engine::DIRTY.store(true, Ordering::Release);
                }
            }
            Command::SetDeviceVolume { input, volume } => {
                let v = if volume.is_finite() { volume.clamp(0.0, 1.0) } else { return Err("Invalid volume".into()) };
                let dev = if input { self.default_in } else { self.default_out };
                if audio {
                    ca::set_volume(dev, input, v);
                }
                *(if input { &mut self.in_volume } else { &mut self.out_volume }) = Some(v);
            }
            Command::SetDeviceMuted { input, muted } => {
                let dev = if input { self.default_in } else { self.default_out };
                if audio {
                    ca::set_muted(dev, input, muted);
                }
                *(if input { &mut self.in_muted } else { &mut self.out_muted }) = Some(muted);
            }
            Command::SetOutputBalance { balance } => {
                let b = if balance.is_finite() { balance.clamp(-1.0, 1.0) } else { return Err("Invalid balance".into()) };
                if audio {
                    ca::set_balance(self.default_out, b);
                }
                self.out_balance = Some(b);
            }
            Command::SetLatency { mode } => {
                self.prefs.latency = mode;
                self.mark_prefs();
            }
            Command::SetShowOtherApps { show } => {
                self.prefs.show_other_apps = show;
                self.prefs_changed = Some(Instant::now());
            }
            Command::ResetAll => {
                for p in self.prefs.apps.values_mut() {
                    *p = AppPrefs { favorite: p.favorite, name: std::mem::take(&mut p.name), ..Default::default() };
                }
                self.prefs.devices.clear();
                self.mark_prefs();
            }
        }
        Ok(None)
    }

    /// Everything the interface needs to draw one frame.
    pub fn snapshot(&mut self) -> Snapshot {
        let info = |d: &ca::Device| DeviceInfo { id: d.id, uid: d.uid.clone(), name: d.name.clone(), transport: d.transport_label() };
        let output = self.device(self.default_out).cloned().map(|d| {
            let mut detail = format!("{} kHz · {} channels · {}", trim_khz(d.sample_rate), d.out_channels, d.transport_label());
            if let Some(r) = self.engine.as_ref().and_then(|e| e.report(&format!("device:{}", d.uid))) {
                detail.push_str(&format!(" · effects add {:.0} ms", r.latency_ms));
            }
            OutputState {
                volume: self.out_volume,
                muted: self.out_muted,
                balance: self.out_balance,
                detail,
                effects: self.prefs.devices.get(&d.uid).cloned().unwrap_or_default(),
                uid: d.uid,
            }
        });
        let apps: Vec<AppState> = self.apps.clone().iter().map(|a| self.app_state(a)).collect();
        let others: Vec<AppState> = self.others.clone().iter().map(|a| self.app_state(a)).collect();
        let mut messages: Vec<String> = self.global_error.iter().cloned().collect();
        if let Some(e) = &self.engine {
            messages.extend(e.errors.iter().filter(|(k, _)| k.starts_with("device:")).map(|(_, m)| format!("Output effects: {m}")));
        }
        let status = match &self.engine {
            None => "Preview".to_string(),
            Some(e) if e.active_count() == 0 => "No apps adjusted".to_string(),
            Some(e) if e.active_count() == 1 => "1 app adjusted".to_string(),
            Some(e) => format!("{} apps adjusted", e.active_count()),
        };
        Snapshot {
            preview: self.engine.is_none(),
            status,
            outputs: self.outputs().iter().map(info).collect(),
            inputs: self.inputs().iter().map(info).collect(),
            default_output: self.default_out,
            default_input: self.default_in,
            default_effects: self.default_fx,
            output,
            input: InputState { volume: self.in_volume, muted: self.in_muted },
            apps,
            others,
            latency: self.prefs.latency,
            latency_options: LATENCY_MODES.iter().map(|&mode| LatencyOption { mode, label: mode.label() }).collect(),
            show_other_apps: self.prefs.show_other_apps,
            max_app_volume: MAX_APP_VOLUME,
            messages,
        }
    }

    fn app_state(&mut self, a: &AppEntry) -> AppState {
        let mut prefs = self.prefs.apps.get(&a.key).cloned().unwrap_or_else(|| AppPrefs { name: a.name.clone(), ..Default::default() });
        // never show (or send back) a corrupt value
        prefs.sanitize();
        let tap = format!("app:{}", a.key);
        let report = self.engine.as_ref().and_then(|e| e.report(&tap));
        let meters = report.as_ref().map(|r| r.stats.meters()).unwrap_or((0.0, 0.0));
        let shown = self.shown_meters.entry(a.key.clone()).or_insert((0.0, 0.0));
        *shown = (decay_meter(shown.0, meters.0), decay_meter(shown.1, meters.1));
        let detail = report.as_ref().map(|r| {
            let mut t = format!("Through SoundMixer · {:.0} ms · {} kHz", r.latency_ms, trim_khz(r.sample_rate));
            if r.escalated {
                t.push_str(" · buffer enlarged to stop glitches");
            }
            t
        });
        AppState {
            key: a.key.clone(),
            name: a.name.clone(),
            bundle_id: (!a.key.starts_with("name:")).then(|| a.key.clone()),
            playing: a.playing,
            running: !a.objects.is_empty(),
            meter: [shown.0, shown.1],
            tapped: report.is_some(),
            detail,
            error: self.engine.as_ref().and_then(|e| e.errors.get(&tap).cloned()),
            prefs,
        }
    }

    /// Save and stop every tap (app quitting).
    pub fn shutdown(&mut self) {
        if let Some(e) = &mut self.engine {
            self.prefs.save();
            e.shutdown();
        }
    }
}

/// 44100 → "44.1", 48000 → "48".
fn trim_khz(hz: f64) -> String {
    let s = format!("{:.1}", hz / 1000.0);
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

/// Meter ballistics: jump up instantly, fall back by 15 % per update. Bad input reads as silence.
fn decay_meter(shown: f32, now: f32) -> f32 {
    let shown = if shown.is_finite() { shown.max(0.0) } else { 0.0 };
    let now = if now.is_finite() { now.max(0.0) } else { 0.0 };
    now.max(shown * 0.85)
}
