//! Core tests: commands and snapshots without any audio hardware (no engine, no saving).

use super::*;

fn device(id: ObjId, name: &str, outs: u32, ins: u32) -> ca::Device {
    ca::Device {
        id,
        uid: format!("uid{id}"),
        name: name.into(),
        out_channels: outs,
        in_channels: ins,
        transport: 0,
        sample_rate: 48000.0,
        hidden: false,
    }
}

fn entry(key: &str, name: &str, playing: bool) -> AppEntry {
    AppEntry {
        key: key.into(),
        name: name.into(),
        objects: if playing { vec![1] } else { vec![] },
        playing,
        last_playing: None,
        regular: true,
    }
}

fn core() -> Core {
    let mut c = Core::with_prefs(Prefs::default(), None);
    c.devices = vec![device(1, "Speakers", 2, 0), device(2, "Mic", 0, 1), device(3, "rs.soundmixer.agg", 2, 0)];
    c.devices[2].uid = "rs.soundmixer.private".into();
    c.default_out = 1;
    c.default_in = 2;
    c.apps = vec![entry("com.spotify.client", "Spotify", true), entry("name:helper", "helper", false)];
    c
}

fn cmd(c: &mut Core, json: &str) -> Result<Option<String>, String> {
    c.apply(serde_json::from_str(json).expect(json))
}

#[test]
fn meter_ballistics() {
    assert_eq!(decay_meter(0.0, 0.5), 0.5, "rises instantly");
    assert!((decay_meter(1.0, 0.0) - 0.85).abs() < 1e-6, "falls 15 % per update");
    assert_eq!(decay_meter(f32::NAN, 0.2), 0.2);
    assert_eq!(decay_meter(0.3, f32::NAN), 0.3 * 0.85);
    assert_eq!(decay_meter(f32::INFINITY, f32::NEG_INFINITY), 0.0);
    assert_eq!(decay_meter(-1.0, -1.0), 0.0);
    let m = (0..200).fold(1.0, |m, _| decay_meter(m, 0.0));
    assert!((0.0..1e-10).contains(&m), "settles to silence");
}

#[test]
fn khz_labels() {
    assert_eq!(trim_khz(44100.0), "44.1");
    assert_eq!(trim_khz(48000.0), "48");
    assert_eq!(trim_khz(0.0), "0");
}

#[test]
fn snapshot_lists_devices_apps_and_hides_private_devices() {
    let mut c = core();
    let s = c.snapshot();
    assert!(s.preview);
    assert_eq!(s.status, "Preview");
    assert_eq!(s.outputs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Speakers"], "own aggregate devices are hidden");
    assert_eq!(s.inputs.len(), 1);
    assert_eq!(s.output.as_ref().unwrap().detail, "48 kHz · 2 channels · Other");
    assert_eq!(s.apps[0].bundle_id.as_deref(), Some("com.spotify.client"));
    assert_eq!(s.apps[1].bundle_id, None, "name: keys have no bundle id");
    assert!(s.apps[0].running && !s.apps[1].running);
    assert_eq!(s.latency_options.len(), 4);
    serde_json::to_string(&s).unwrap();
}

#[test]
fn set_app_sanitises_and_flags_reconcile() {
    let mut c = core();
    c.need_reconcile = false;
    cmd(&mut c, r#"{"cmd":"set_app","key":"a","prefs":{"volume":99,"balance":-7}}"#).unwrap();
    let a = &c.prefs.apps["a"];
    assert_eq!(a.volume, MAX_APP_VOLUME);
    assert_eq!(a.balance, -1.0);
    assert!(c.need_reconcile, "effects went from neutral to active");
    assert!(c.prefs_changed.is_some());

    c.need_reconcile = false;
    cmd(&mut c, r#"{"cmd":"set_app","key":"a","prefs":{"volume":2,"balance":0}}"#).unwrap();
    assert!(!c.need_reconcile, "a slider move only updates the running chain");
}

#[test]
fn corrupt_saved_values_never_reach_the_snapshot() {
    let mut c = core();
    c.prefs.apps.insert("com.spotify.client".into(), AppPrefs { volume: f32::NAN, balance: f32::INFINITY, ..Default::default() });
    let s = c.snapshot();
    assert_eq!(s.apps[0].prefs.volume, 1.0);
    assert_eq!(s.apps[0].prefs.balance, 0.0);
    serde_json::to_string(&s).expect("no NaN in JSON");
}

#[test]
fn reset_all_keeps_favourites_and_names() {
    let mut c = core();
    c.prefs.apps.insert("a".into(), AppPrefs { volume: 3.0, favorite: true, name: "A".into(), boost: true, ..Default::default() });
    c.prefs.devices.insert("uid1".into(), DevicePrefs { boost: true, ..Default::default() });
    cmd(&mut c, r#"{"cmd":"reset_all"}"#).unwrap();
    let a = &c.prefs.apps["a"];
    assert_eq!((a.volume, a.boost, a.favorite, a.name.as_str()), (1.0, false, true, "A"));
    assert!(c.prefs.devices.is_empty());
}

#[test]
fn device_commands_clamp_and_reject_bad_numbers() {
    let mut c = core();
    cmd(&mut c, r#"{"cmd":"set_device_volume","input":false,"volume":7}"#).unwrap();
    assert_eq!(c.out_volume, Some(1.0));
    cmd(&mut c, r#"{"cmd":"set_device_volume","input":true,"volume":-1}"#).unwrap();
    assert_eq!(c.in_volume, Some(0.0));
    cmd(&mut c, r#"{"cmd":"set_output_balance","balance":3}"#).unwrap();
    assert_eq!(c.out_balance, Some(1.0));
    cmd(&mut c, r#"{"cmd":"set_device_muted","input":false,"muted":true}"#).unwrap();
    assert_eq!(c.out_muted, Some(true));
}

#[test]
fn load_profile_enables_it_and_reports_count() {
    let mut c = core();
    let text = "Preamp: -6 dB\nFilter 1: ON PK Fc 100 Hz Gain 3 dB Q 1.0\nFilter 2: ON HSC Fc 8000 Hz Gain -2 dB Q 0.7\n";
    let msg = cmd(&mut c, &serde_json::json!({"cmd":"load_profile","uid":"uid1","text":text}).to_string()).unwrap();
    assert_eq!(msg.as_deref(), Some("Loaded 2 filters"));
    let d = &c.prefs.devices["uid1"];
    assert!(d.peq.enabled && d.peq_name.is_some());
    assert!(cmd(&mut c, r#"{"cmd":"load_profile","uid":"uid1","text":"garbage"}"#).is_err());
    assert_eq!(c.prefs.devices["uid1"].peq.count, 2, "a bad paste keeps the old profile");
}

#[test]
fn device_prefs_round_trip_through_the_snapshot() {
    let mut c = core();
    let mut d = DevicePrefs { boost: true, ..Default::default() };
    d.eq.enabled = true;
    d.eq.bands[3] = 0.5;
    c.prefs.devices.insert("uid1".into(), d.clone());
    let json = serde_json::to_value(c.snapshot()).unwrap();
    let back = serde_json::json!({"cmd":"set_device","uid":"uid1","prefs":json["output"]["effects"]});
    cmd(&mut c, &back.to_string()).unwrap();
    assert_eq!(c.prefs.devices["uid1"], d);
}

#[test]
fn preview_changes_nothing_on_disk_or_hardware() {
    let mut c = core();
    cmd(&mut c, r#"{"cmd":"set_default_device","role":"output","id":42}"#).unwrap();
    assert_eq!(c.default_out, 1, "preview never switches devices");
    cmd(&mut c, r#"{"cmd":"set_latency","mode":"Low"}"#).unwrap();
    c.prefs_changed = Some(Instant::now() - Duration::from_secs(5));
    c.tick();
    assert!(c.prefs_changed.is_some(), "preview never saves");
}
