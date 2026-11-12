//! Saved settings (per app, per output device, and global).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::audio::dsp::{ChainSettings, Eq10Settings, PeqSettings, PEQ_MAX};
use crate::audio::engine::LatencyMode;

/// Highest per-app volume (4 = 400 %, estimated). Above 100 % the peak limiter keeps it clean.
pub const MAX_APP_VOLUME: f32 = 4.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPrefs {
    pub volume: f32,
    pub muted: bool,
    pub boost: bool,
    pub balance: f32,
    pub eq: Eq10Settings,
    /// Output device UID; None = match the system output device.
    pub output: Option<String>,
    pub favorite: bool,
    pub name: String,
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            volume: 1.0,
            muted: false,
            boost: false,
            balance: 0.0,
            eq: Eq10Settings::default(),
            output: None,
            favorite: false,
            name: String::new(),
        }
    }
}

/// `v` if finite, else `fallback`, then clamped.
fn finite(v: f32, fallback: f32, lo: f32, hi: f32) -> f32 {
    (if v.is_finite() { v } else { fallback }).clamp(lo, hi)
}

fn sanitize_eq(eq: &mut Eq10Settings) {
    eq.preamp = finite(eq.preamp, 0.0, -1.0, 1.0);
    for b in &mut eq.bands {
        *b = finite(*b, 0.0, -1.0, 1.0);
    }
}

fn sanitize_peq(p: &mut PeqSettings) {
    p.count = p.count.min(PEQ_MAX);
    p.preamp_db = finite(p.preamp_db, 0.0, -60.0, 30.0);
    for b in &mut p.bands {
        b.freq = finite(b.freq, 1000.0, 1.0, 96_000.0);
        b.gain_db = finite(b.gain_db, 0.0, -60.0, 30.0);
        b.q = finite(b.q, 0.707, 0.05, 100.0);
    }
}

impl AppPrefs {
    /// Bring every value into its valid range (settings files can be edited or corrupted).
    pub fn sanitize(&mut self) {
        // a broken volume becomes "unchanged", never silence or full boost
        self.volume = finite(self.volume, 1.0, 0.0, MAX_APP_VOLUME);
        self.balance = finite(self.balance, 0.0, -1.0, 1.0);
        sanitize_eq(&mut self.eq);
    }

    pub fn chain(&self) -> ChainSettings {
        ChainSettings {
            volume: self.volume,
            muted: self.muted,
            boost: self.boost,
            balance: self.balance,
            eq: self.eq,
            peq: PeqSettings::default(),
        }
    }
    pub fn effects_neutral(&self) -> bool {
        self.chain().is_neutral()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DevicePrefs {
    pub eq: Eq10Settings,
    pub peq: PeqSettings,
    pub peq_name: Option<String>,
    pub boost: bool,
}

impl DevicePrefs {
    pub fn sanitize(&mut self) {
        sanitize_eq(&mut self.eq);
        sanitize_peq(&mut self.peq);
    }

    pub fn chain(&self) -> ChainSettings {
        ChainSettings { volume: 1.0, muted: false, boost: self.boost, balance: 0.0, eq: self.eq, peq: self.peq }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub apps: BTreeMap<String, AppPrefs>,
    pub devices: BTreeMap<String, DevicePrefs>,
    pub latency: LatencyMode,
    pub show_other_apps: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { apps: BTreeMap::new(), devices: BTreeMap::new(), latency: LatencyMode::Auto, show_other_apps: false }
    }
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("SoundMixer").join("settings.json"))
}

impl Prefs {
    pub fn sanitize(&mut self) {
        self.apps.values_mut().for_each(AppPrefs::sanitize);
        self.devices.values_mut().for_each(DevicePrefs::sanitize);
    }

    /// Parse a settings file. Tolerant: an entry that cannot be read is dropped on its own instead
    /// of throwing away every setting, and all values are brought into range.
    pub fn from_json(text: &str) -> Self {
        let mut p = Prefs::default();
        let Ok(serde_json::Value::Object(root)) = serde_json::from_str::<serde_json::Value>(text) else { return p };
        fn map<T: serde::de::DeserializeOwned>(v: Option<&serde_json::Value>) -> BTreeMap<String, T> {
            let Some(serde_json::Value::Object(m)) = v else { return BTreeMap::new() };
            m.iter().filter_map(|(k, v)| serde_json::from_value(v.clone()).ok().map(|t| (k.clone(), t))).collect()
        }
        p.apps = map(root.get("apps"));
        p.devices = map(root.get("devices"));
        if let Some(l) = root.get("latency").and_then(|v| serde_json::from_value(v.clone()).ok()) {
            p.latency = l;
        }
        if let Some(b) = root.get("show_other_apps").and_then(|v| v.as_bool()) {
            p.show_other_apps = b;
        }
        p.sanitize();
        p
    }

    /// Serialise for saving. Values are sanitised first: serde_json writes NaN as `null`, which
    /// would make the file unreadable on the next launch.
    pub fn to_json(&self) -> String {
        let mut p = self.clone();
        p.sanitize();
        serde_json::to_string_pretty(&p).unwrap_or_else(|_| "{}".into())
    }

    pub fn load() -> Self {
        path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Self::from_json(&t)).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, self.to_json()).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::dsp::{FilterKind, PeqBand};

    #[test]
    fn defaults_are_neutral() {
        let a = AppPrefs::default();
        assert!(a.effects_neutral());
        assert_eq!(a.volume, 1.0);
        assert_eq!(Prefs::default().latency, LatencyMode::Auto);
    }

    #[test]
    fn sanitize_clamps_app_values() {
        for (v, want) in [
            (f32::NAN, 1.0),
            (f32::INFINITY, 1.0),
            (f32::NEG_INFINITY, 1.0),
            (-2.0, 0.0),
            (99.0, MAX_APP_VOLUME),
            (0.0, 0.0),
            (MAX_APP_VOLUME, MAX_APP_VOLUME),
            (-0.0, 0.0),
        ] {
            let mut a = AppPrefs { volume: v, ..Default::default() };
            a.sanitize();
            assert_eq!(a.volume, want, "volume {v}");
        }
        for (b, want) in [(f32::NAN, 0.0), (-7.0, -1.0), (7.0, 1.0), (0.3, 0.3)] {
            let mut a = AppPrefs { balance: b, ..Default::default() };
            a.sanitize();
            assert_eq!(a.balance, want, "balance {b}");
        }
        let mut a = AppPrefs::default();
        a.eq.bands = [f32::NAN, 5.0, -5.0, 0.5, f32::INFINITY, 0.0, 0.0, 0.0, 0.0, -1.0];
        a.eq.preamp = f32::NEG_INFINITY;
        a.sanitize();
        assert_eq!(a.eq.bands, [0.0, 1.0, -1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0]);
        assert_eq!(a.eq.preamp, 0.0);
    }

    #[test]
    fn sanitize_caps_peq_count_so_is_neutral_cannot_panic() {
        let mut d = DevicePrefs::default();
        d.peq.enabled = true;
        d.peq.count = 10_000;
        d.sanitize();
        assert_eq!(d.peq.count, PEQ_MAX);
        let _ = d.chain().is_neutral();
    }

    #[test]
    fn sanitize_peq_bands() {
        let mut d = DevicePrefs::default();
        d.peq.count = 3;
        d.peq.preamp_db = f32::NAN;
        d.peq.bands[0] = PeqBand { kind: FilterKind::Peak, freq: f32::NAN, gain_db: f32::INFINITY, q: 0.0 };
        d.peq.bands[1] = PeqBand { kind: FilterKind::LowShelf, freq: -10.0, gain_db: -500.0, q: f32::NAN };
        d.peq.bands[2] = PeqBand { kind: FilterKind::HighShelf, freq: 1e9, gain_db: 3.0, q: 1e6 };
        d.sanitize();
        assert_eq!(d.peq.preamp_db, 0.0);
        assert_eq!((d.peq.bands[0].freq, d.peq.bands[0].gain_db, d.peq.bands[0].q), (1000.0, 0.0, 0.05));
        assert_eq!((d.peq.bands[1].freq, d.peq.bands[1].gain_db, d.peq.bands[1].q), (1.0, -60.0, 0.707));
        assert_eq!((d.peq.bands[2].freq, d.peq.bands[2].q), (96_000.0, 100.0));
    }

    #[test]
    fn nan_survives_a_save_load_round_trip() {
        let mut p = Prefs::default();
        p.apps.insert("a".into(), AppPrefs { volume: f32::NAN, favorite: true, ..Default::default() });
        p.apps.insert("b".into(), AppPrefs { volume: 2.5, ..Default::default() });
        let back = Prefs::from_json(&p.to_json());
        assert_eq!(back.apps.len(), 2, "nothing lost");
        assert_eq!(back.apps["a"].volume, 1.0);
        assert!(back.apps["a"].favorite);
        assert_eq!(back.apps["b"].volume, 2.5);
    }

    #[test]
    fn round_trip_is_lossless_for_valid_prefs() {
        let mut p = Prefs { latency: LatencyMode::Safe, show_other_apps: true, ..Default::default() };
        p.apps.insert(
            "com.apple.Safari".into(),
            AppPrefs {
                volume: 0.4,
                muted: true,
                boost: true,
                balance: -0.5,
                output: Some("uid".into()),
                favorite: true,
                name: "Safari".into(),
                ..Default::default()
            },
        );
        let mut d = DevicePrefs { boost: true, peq_name: Some("HD 600".into()), ..Default::default() };
        d.eq.enabled = true;
        d.eq.bands[3] = 0.25;
        p.devices.insert("dev".into(), d);
        assert_eq!(Prefs::from_json(&p.to_json()), p);
    }

    #[test]
    fn one_bad_entry_does_not_wipe_the_rest() {
        let t = r#"{
            "apps": {
                "good": {"volume": 0.5, "name": "Good"},
                "bad":  {"volume": "loud"},
                "worse": 42
            },
            "devices": {"d": {"boost": "yes"}, "ok": {"boost": true}},
            "latency": "NotAMode",
            "show_other_apps": "maybe"
        }"#;
        let p = Prefs::from_json(t);
        assert_eq!(p.apps.len(), 1);
        assert_eq!(p.apps["good"].volume, 0.5);
        assert_eq!(p.devices.len(), 1);
        assert!(p.devices["ok"].boost);
        assert_eq!(p.latency, LatencyMode::Auto, "unknown mode falls back");
        assert!(!p.show_other_apps);
    }

    #[test]
    fn hostile_files_give_defaults() {
        for t in
            ["", " ", "null", "[]", "42", "\"str\"", "{", "{\"apps\": []}", "{\"apps\": null}", "\u{feff}{}", "{\"apps\": {\"x\": {}}}"]
        {
            let p = Prefs::from_json(t);
            assert!(p.devices.is_empty(), "{t:?}");
            assert!(p.apps.values().all(|a| a.effects_neutral()), "{t:?}");
        }
        // an empty app object is a valid app with defaults
        assert_eq!(Prefs::from_json("{\"apps\": {\"x\": {}}}").apps["x"], AppPrefs::default());
    }

    #[test]
    fn unknown_fields_and_missing_fields_are_fine() {
        let p = Prefs::from_json(r#"{"apps":{"a":{"volume":3.0,"future_field":[1,2]}},"version":7}"#);
        assert_eq!(p.apps["a"].volume, 3.0);
        assert_eq!(p.apps["a"].balance, 0.0);
    }

    #[test]
    fn out_of_range_json_numbers_are_clamped() {
        let p = Prefs::from_json(
            r#"{"apps":{"a":{"volume":1e30,"balance":-1e30,"eq":{"enabled":true,"preamp":9,"bands":[9,9,9,9,9,-9,-9,-9,-9,-9]}}}}"#,
        );
        let a = &p.apps["a"];
        assert_eq!(a.volume, MAX_APP_VOLUME);
        assert_eq!(a.balance, -1.0);
        assert_eq!(a.eq.preamp, 1.0);
        assert_eq!(a.eq.bands, [1.0, 1.0, 1.0, 1.0, 1.0, -1.0, -1.0, -1.0, -1.0, -1.0]);
        // f32 overflow in JSON (1e300) is rejected by serde for that entry only
        let p = Prefs::from_json(r#"{"apps":{"x":{"volume":1e300},"y":{}}}"#);
        assert!(p.apps.contains_key("y"));
        assert!(p.apps.get("x").map(|a| a.volume.is_finite()).unwrap_or(true));
    }

    #[test]
    fn many_apps_and_long_keys() {
        let mut p = Prefs::default();
        for i in 0..5000 {
            p.apps.insert(format!("{}{i}", "k".repeat(200)), AppPrefs { volume: (i % 500) as f32 / 100.0, ..Default::default() });
        }
        let back = Prefs::from_json(&p.to_json());
        assert_eq!(back.apps.len(), 5000);
        assert!(back.apps.values().all(|a| (0.0..=MAX_APP_VOLUME).contains(&a.volume)));
    }
}
