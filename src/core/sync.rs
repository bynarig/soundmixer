//! Keeps the device list, app list and saved settings in sync with Core Audio and tells the
//! engine which taps to run.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{AppEntry, Core, RECENT};
use crate::audio::ca::{self, ObjId, Role};
use crate::audio::engine::{self, SharedChain, TapSpec};

pub(super) fn identify(pid: i32, fallback_bundle: &str) -> (String, String, bool) {
    let rp = ca::responsible_pid(pid);
    #[allow(unused_unsafe)]
    let ns = unsafe { objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(rp) };
    if let Some(app) = ns {
        #[allow(unused_unsafe)]
        let (bundle, name, regular) = unsafe {
            (
                app.bundleIdentifier().map(|s| s.to_string()),
                app.localizedName().map(|s| s.to_string()),
                app.activationPolicy() == objc2_app_kit::NSApplicationActivationPolicy::Regular,
            )
        };
        if let Some(b) = bundle {
            return (b.clone(), name.unwrap_or(b), regular);
        }
        if let Some(n) = name {
            return (format!("name:{n}"), n, regular);
        }
    }
    if !fallback_bundle.is_empty() {
        let short = fallback_bundle.rsplit('.').next().unwrap_or(fallback_bundle).to_string();
        return (fallback_bundle.to_string(), short, false);
    }
    let n = ca::pid_name(rp);
    (format!("name:{n}"), n, false)
}

impl Core {
    pub(super) fn device(&self, id: ObjId) -> Option<&ca::Device> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub(super) fn device_uid(&self, uid: &str) -> Option<&ca::Device> {
        self.devices.iter().find(|d| d.uid == uid)
    }

    pub(super) fn outputs(&self) -> Vec<ca::Device> {
        self.devices.iter().filter(|d| d.out_channels > 0 && !d.hidden && !d.uid.starts_with("rs.soundmixer.")).cloned().collect()
    }

    pub(super) fn inputs(&self) -> Vec<ca::Device> {
        self.devices.iter().filter(|d| d.in_channels > 0 && !d.hidden && !d.uid.starts_with("rs.soundmixer.")).cloned().collect()
    }

    pub(super) fn mark_prefs(&mut self) {
        self.prefs_changed = Some(Instant::now());
        self.need_reconcile = true;
    }

    // ------------------------------------------------------------ polling

    pub(super) fn refresh(&mut self, full: bool) {
        let now = Instant::now();
        if full {
            self.devices = ca::devices();
            self.default_out = ca::default_device(Role::Output);
            self.default_in = ca::default_device(Role::Input);
            self.default_fx = ca::default_device(Role::SoundEffects);
            if let Some(e) = &mut self.engine {
                e.listen(&self.devices);
            }
        }
        // group audio processes by the app responsible for them (helpers belong to their app)
        let procs = ca::processes();
        let mut groups: HashMap<String, AppEntry> = HashMap::new();
        for p in procs {
            if p.pid == self.own_pid || p.pid <= 0 {
                continue;
            }
            let (key, name, regular) = match self.ident_cache.get(&p.pid) {
                Some(v) => v.clone(),
                None => {
                    let v = identify(p.pid, &p.bundle_id);
                    self.ident_cache.insert(p.pid, v.clone());
                    v
                }
            };
            if key.starts_with("com.apple.audio.") || key == "com.apple.controlcenter" {
                continue;
            }
            let e = groups.entry(key.clone()).or_insert(AppEntry {
                key,
                name,
                objects: Vec::new(),
                playing: false,
                last_playing: None,
                regular,
            });
            e.objects.push(p.object);
            e.playing |= p.running_output;
        }
        let previous: HashMap<String, Option<Instant>> =
            self.apps.iter().chain(self.others.iter()).map(|a| (a.key.clone(), a.last_playing)).collect();
        let mut shown = Vec::new();
        let mut others = Vec::new();
        for (_, mut e) in groups {
            e.objects.sort_unstable();
            e.last_playing = if e.playing { Some(now) } else { previous.get(&e.key).copied().flatten() };
            let p = self.prefs.apps.get(&e.key);
            let recent = e.last_playing.map(|t| now.duration_since(t) < RECENT).unwrap_or(false);
            let keep = e.playing || recent || p.map(|p| p.favorite || !p.effects_neutral() || p.output.is_some()).unwrap_or(false);
            if let Some(p) = self.prefs.apps.get_mut(&e.key) {
                if p.name != e.name {
                    p.name = e.name.clone();
                }
            }
            if keep {
                shown.push(e);
            } else if e.regular {
                others.push(e);
            }
        }
        // favourites that are not running stay listed
        for (k, p) in &self.prefs.apps {
            if p.favorite && !shown.iter().any(|a| &a.key == k) {
                others.retain(|o| &o.key != k);
                shown.push(AppEntry {
                    key: k.clone(),
                    name: if p.name.is_empty() { k.clone() } else { p.name.clone() },
                    objects: Vec::new(),
                    playing: false,
                    last_playing: None,
                    regular: true,
                });
            }
        }
        shown.sort_by(|a, b| {
            let fa = self.prefs.apps.get(&a.key).map(|p| p.favorite).unwrap_or(false);
            let fb = self.prefs.apps.get(&b.key).map(|p| p.favorite).unwrap_or(false);
            fb.cmp(&fa).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        others.sort_by_key(|a| a.name.to_lowercase());
        if self.apps.iter().map(|a| (&a.key, &a.objects)).ne(shown.iter().map(|a| (&a.key, &a.objects))) {
            self.need_reconcile = true;
        }
        self.apps = shown;
        self.others = others;
        if self.ident_cache.len() > 2000 {
            self.ident_cache.clear();
        }
        self.last_refresh = Some(now);
    }

    pub(super) fn refresh_volumes(&mut self) {
        self.out_volume = ca::volume(self.default_out, false);
        self.out_muted = ca::muted(self.default_out, false);
        self.out_balance = ca::balance(self.default_out);
        self.in_volume = ca::volume(self.default_in, true);
        self.in_muted = ca::muted(self.default_in, true);
        self.last_volumes = Some(Instant::now());
    }

    pub(super) fn app_chain(&mut self, key: &str) -> Arc<SharedChain> {
        let s = self.prefs.apps.get(key).cloned().unwrap_or_default().chain();
        let c = self.app_chains.entry(key.to_string()).or_insert_with(|| SharedChain::new(s)).clone();
        c.set(s);
        c
    }

    pub(super) fn dev_chain(&mut self, uid: &str) -> Option<Arc<SharedChain>> {
        let p = self.prefs.devices.get(uid)?.clone();
        let s = p.chain();
        if s.is_neutral() {
            return None;
        }
        let c = self.dev_chains.entry(uid.to_string()).or_insert_with(|| SharedChain::new(s)).clone();
        c.set(s);
        Some(c)
    }

    /// Push slider changes to the audio thread (cheap: only changed chains bump a counter).
    pub(super) fn push_params(&mut self) {
        let keys: Vec<String> = self.app_chains.keys().cloned().collect();
        for k in keys {
            let s = self.prefs.apps.get(&k).cloned().unwrap_or_default().chain();
            self.app_chains[&k].set(s);
        }
        let uids: Vec<String> = self.dev_chains.keys().cloned().collect();
        for u in uids {
            let s = self.prefs.devices.get(&u).cloned().unwrap_or_default().chain();
            self.dev_chains[&u].set(s);
        }
    }

    pub(super) fn build_specs(&mut self) -> Vec<TapSpec> {
        let Some(def) = self.device(self.default_out).cloned() else { return Vec::new() };
        let latency = self.prefs.latency;
        let mut specs = Vec::new();
        let mut tapped: Vec<ObjId> = Vec::new();
        let apps = self.apps.clone();
        for a in apps.iter().filter(|a| !a.objects.is_empty()) {
            let p = self.prefs.apps.get(&a.key).cloned().unwrap_or_default();
            // a chosen device that is unplugged falls back to the system output device
            let target =
                p.output.as_deref().and_then(|u| self.device_uid(u).cloned()).filter(|d| d.out_channels > 0).unwrap_or_else(|| def.clone());
            let routed = target.uid != def.uid;
            if p.effects_neutral() && !routed {
                continue;
            }
            let app_chain = (!p.effects_neutral()).then(|| self.app_chain(&a.key));
            let dev_chain = self.dev_chain(&target.uid);
            tapped.extend(a.objects.iter().copied());
            specs.push(TapSpec {
                key: format!("app:{}", a.key),
                label: a.name.clone(),
                processes: a.objects.clone(),
                global: false,
                out_uid: target.uid.clone(),
                frames: latency.frames_for(&target),
                app: app_chain,
                dev: dev_chain,
            });
        }
        self.global_error = None;
        if let Some(dc) = self.dev_chain(&def.uid) {
            let own = ca::process_object_for_pid(self.own_pid);
            if own == 0 {
                self.global_error = Some("Output effects are waiting for macOS to register this app.".into());
            } else {
                let mut excl = tapped;
                excl.push(own);
                excl.sort_unstable();
                excl.dedup();
                specs.push(TapSpec {
                    key: format!("device:{}", def.uid),
                    label: format!("{} effects", def.name),
                    processes: excl,
                    global: true,
                    out_uid: def.uid.clone(),
                    frames: latency.frames_for(&def),
                    app: None,
                    dev: Some(dc),
                });
            }
        }
        specs
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        let dirty = engine::DIRTY.swap(false, std::sync::atomic::Ordering::AcqRel);
        if dirty || self.last_refresh.map(|t| now.duration_since(t) > Duration::from_millis(700)).unwrap_or(true) {
            self.refresh(dirty || self.last_refresh.map(|t| now.duration_since(t) > Duration::from_secs(5)).unwrap_or(true));
            if dirty {
                self.need_reconcile = true;
            }
        }
        if self.last_volumes.map(|t| now.duration_since(t) > Duration::from_millis(400)).unwrap_or(true) {
            self.refresh_volumes();
        }
        self.push_params();
        if self.engine.is_some() {
            if self.need_reconcile {
                self.need_reconcile = false;
                let specs = self.build_specs();
                if let Some(e) = &mut self.engine {
                    e.reconcile(specs, false);
                }
            } else if let Some(e) = &mut self.engine {
                e.reconcile_retiring_only();
                e.watchdog();
            }
        }
        // a preview (no engine) never writes the person's settings
        if let (Some(t), true) = (self.prefs_changed, self.engine.is_some()) {
            if now.duration_since(t) > Duration::from_millis(800) {
                self.prefs.save();
                self.prefs_changed = None;
            }
        }
    }
}
