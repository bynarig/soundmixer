//! The audio engine: one muted Core Audio process tap + private aggregate device per adjusted app
//! (and one global tap for output-device effects). Untouched apps are never tapped, so they keep
//! their native, zero-added-latency path ("on-demand taps").

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_core_audio::*;
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioTimeStamp};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};

use crate::audio::ca::{self, ObjId};
use crate::audio::dsp::{self, Chain, ChainSettings, FADE_S, MAX_FRAMES};

/// Set by every Core Audio listener; the UI thread reconciles on its next tick.
pub static DIRTY: AtomicBool = AtomicBool::new(true);

/// Callbacks slower than this share of the buffer period count as "late" (estimated).
pub const LATE_RATIO: f64 = 0.5;
/// Late callbacks or HAL overloads within the window that trigger a bigger buffer.
pub const ESCALATE_AFTER: u32 = 3;
pub const ESCALATE_WINDOW: Duration = Duration::from_secs(2);
pub const MAX_ESCALATED_FRAMES: u32 = 2048;
/// How long a replaced tap keeps running (fading out) before it is destroyed.
pub const RETIRE_AFTER: Duration = Duration::from_millis(80);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LatencyMode {
    /// Chosen from the device's connection type.
    Auto,
    Low,
    Balanced,
    Safe,
}

impl LatencyMode {
    pub fn label(&self) -> &'static str {
        match self {
            LatencyMode::Auto => "Automatic",
            LatencyMode::Low => "Lowest latency",
            LatencyMode::Balanced => "Balanced",
            LatencyMode::Safe => "Glitch-proof",
        }
    }
    /// IO buffer size (frames) for a device.
    pub fn frames_for(&self, d: &ca::Device) -> u32 {
        match self {
            LatencyMode::Low => 128,
            LatencyMode::Balanced => 256,
            LatencyMode::Safe => 1024,
            LatencyMode::Auto => {
                if d.is_bluetooth() || d.is_airplay() {
                    1024
                } else if d.is_aggregate() || d.transport == kAudioDeviceTransportTypeVirtual {
                    512
                } else {
                    256
                }
            }
        }
    }
}

// ------------------------------------------------------------------ shared parameters

/// Settings for one processing chain, shared between the UI and audio threads.
pub struct SharedChain {
    settings: Mutex<ChainSettings>,
    gen: AtomicU64,
}

impl SharedChain {
    pub fn new(s: ChainSettings) -> Arc<Self> {
        Arc::new(Self { settings: Mutex::new(s), gen: AtomicU64::new(1) })
    }
    pub fn set(&self, s: ChainSettings) {
        let mut g = self.settings.lock().unwrap();
        if *g != s {
            *g = s;
            self.gen.fetch_add(1, Ordering::Release);
        }
    }
    pub fn get(&self) -> ChainSettings {
        *self.settings.lock().unwrap()
    }
}

/// Live numbers a tap reports back to the UI.
#[derive(Default)]
pub struct TapStats {
    pub meter_l: AtomicU32,
    pub meter_r: AtomicU32,
    pub late: AtomicU32,
    pub overloads: AtomicU32,
    pub callbacks: AtomicU64,
    /// last callback's processing time as a share of the buffer period, in 1/1000
    pub load_permille: AtomicU32,
    pub fading_out: AtomicBool,
    pub no_input: AtomicU32,
}

impl TapStats {
    pub fn meters(&self) -> (f32, f32) {
        (f32::from_bits(self.meter_l.load(Ordering::Relaxed)), f32::from_bits(self.meter_r.load(Ordering::Relaxed)))
    }
}

// ------------------------------------------------------------------ the IO callback

struct IoContext {
    app: Option<Arc<SharedChain>>,
    dev: Option<Arc<SharedChain>>,
    app_gen: u64,
    dev_gen: u64,
    app_chain: Chain,
    dev_chain: Chain,
    l: Box<[f32]>,
    r: Box<[f32]>,
    sr: f64,
    stereo: (usize, usize),
    fade: f32,
    fade_step: f32,
    stats: Arc<TapStats>,
}

#[inline]
fn flush_denormals() {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let mut fpcr: u64;
        core::arch::asm!("mrs {}, fpcr", out(reg) fpcr);
        if fpcr & (1 << 24) == 0 {
            core::arch::asm!("msr fpcr, {}", in(reg) fpcr | (1 << 24));
        }
    }
    #[cfg(target_arch = "x86_64")]
    #[allow(deprecated)]
    unsafe {
        use core::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        _mm_setcsr(_mm_getcsr() | 0x8040);
    }
}

unsafe fn buffers<'a>(list: NonNull<AudioBufferList>) -> &'a mut [AudioBuffer] {
    let l = list.as_ptr();
    std::slice::from_raw_parts_mut((*l).mBuffers.as_mut_ptr(), (*l).mNumberBuffers as usize)
}

unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _in_time: NonNull<AudioTimeStamp>,
    output: NonNull<AudioBufferList>,
    _out_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> i32 {
    flush_denormals();
    let t0 = Instant::now();
    let ctx = &mut *(client as *mut IoContext);
    let sr = ctx.sr;

    // pick up new settings without ever blocking
    if let Some(a) = &ctx.app {
        let g = a.gen.load(Ordering::Acquire);
        if g != ctx.app_gen {
            if let Ok(s) = a.settings.try_lock() {
                ctx.app_chain.configure(&s, sr);
                ctx.app_gen = g;
            }
        }
    }
    if let Some(d) = &ctx.dev {
        let g = d.gen.load(Ordering::Acquire);
        if g != ctx.dev_gen {
            if let Ok(s) = d.settings.try_lock() {
                ctx.dev_chain.configure(&s, sr);
                ctx.dev_gen = g;
            }
        }
    }

    let outs = buffers(output);
    for b in outs.iter_mut() {
        if !b.mData.is_null() {
            std::ptr::write_bytes(b.mData as *mut u8, 0, b.mDataByteSize as usize);
        }
    }
    let total_out: usize = outs.iter().map(|b| b.mNumberChannels as usize).sum();
    if total_out == 0 {
        return 0;
    }

    // The tap's stream(s) come after any inputs of the output device itself: take the last ones.
    let ins = buffers(input);
    let (src_a, src_b, stride, frames) = match ins.len() {
        0 => {
            ctx.stats.no_input.fetch_add(1, Ordering::Relaxed);
            return 0;
        }
        n => {
            let last = &ins[n - 1];
            let ch = last.mNumberChannels.max(1) as usize;
            let frames = last.mDataByteSize as usize / (4 * ch);
            if ch >= 2 {
                (last.mData as *const f32, (last.mData as *const f32).add(1), ch, frames)
            } else if n >= 2 && ins[n - 2].mNumberChannels == 1 {
                (ins[n - 2].mData as *const f32, last.mData as *const f32, 1, frames)
            } else {
                (last.mData as *const f32, last.mData as *const f32, 1, frames)
            }
        }
    };
    if src_a.is_null() || src_b.is_null() || frames == 0 {
        ctx.stats.no_input.fetch_add(1, Ordering::Relaxed);
        return 0;
    }

    let fading_out = ctx.stats.fading_out.load(Ordering::Relaxed);
    let fade_target = if fading_out { 0.0 } else { 1.0 };
    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    let (cl, cr) = ctx.stereo;
    let mut done = 0usize;
    while done < frames {
        let n = (frames - done).min(MAX_FRAMES);
        let l = &mut ctx.l[..n];
        let r = &mut ctx.r[..n];
        for i in 0..n {
            let k = (done + i) * stride;
            l[i] = *src_a.add(k);
            r[i] = *src_b.add(k);
        }
        if ctx.app.is_some() {
            ctx.app_chain.process(l, r);
        }
        if ctx.dev.is_some() {
            ctx.dev_chain.process(l, r);
        }
        if ctx.fade != fade_target || ctx.fade != 1.0 {
            for i in 0..n {
                if ctx.fade < fade_target {
                    ctx.fade = (ctx.fade + ctx.fade_step).min(fade_target);
                } else if ctx.fade > fade_target {
                    ctx.fade = (ctx.fade - ctx.fade_step).max(fade_target);
                }
                l[i] *= ctx.fade;
                r[i] *= ctx.fade;
            }
        }
        peak_l = peak_l.max(dsp::peak(l));
        peak_r = peak_r.max(dsp::peak(r));

        // write into the device's stereo pair (or mono mix), wherever those channels live
        if total_out == 1 {
            let b = &outs[0];
            let dst = b.mData as *mut f32;
            if !dst.is_null() {
                let cap = b.mDataByteSize as usize / 4;
                for i in 0..n {
                    if done + i < cap {
                        *dst.add(done + i) = 0.5 * (l[i] + r[i]);
                    }
                }
            }
        } else {
            let mut base = 0usize;
            for b in outs.iter() {
                let ch = b.mNumberChannels as usize;
                let dst = b.mData as *mut f32;
                if !dst.is_null() && ch > 0 {
                    let cap = b.mDataByteSize as usize / 4;
                    for (want, src) in [(cl, &*l), (cr, &*r)] {
                        if want >= base && want < base + ch {
                            let off = want - base;
                            for (i, &v) in src[..n].iter().enumerate() {
                                let idx = (done + i) * ch + off;
                                if idx < cap {
                                    *dst.add(idx) = v;
                                }
                            }
                        }
                    }
                }
                base += ch;
            }
        }
        done += n;
    }

    let s = &ctx.stats;
    s.meter_l.store(peak_l.to_bits(), Ordering::Relaxed);
    s.meter_r.store(peak_r.to_bits(), Ordering::Relaxed);
    s.callbacks.fetch_add(1, Ordering::Relaxed);
    let period = frames as f64 / sr;
    let used = t0.elapsed().as_secs_f64();
    s.load_permille.store(((used / period) * 1000.0) as u32, Ordering::Relaxed);
    if used > period * LATE_RATIO {
        s.late.fetch_add(1, Ordering::Relaxed);
    }
    0
}

unsafe extern "C-unwind" fn overload_listener(_id: ObjId, _n: u32, _a: NonNull<AudioObjectPropertyAddress>, data: *mut c_void) -> i32 {
    let stats = &*(data as *const TapStats);
    stats.overloads.fetch_add(1, Ordering::Relaxed);
    0
}

unsafe extern "C-unwind" fn dirty_listener(_id: ObjId, _n: u32, _a: NonNull<AudioObjectPropertyAddress>, _d: *mut c_void) -> i32 {
    DIRTY.store(true, Ordering::Release);
    0
}

// ------------------------------------------------------------------ tap units

#[derive(Clone)]
pub struct TapSpec {
    pub key: String,
    pub label: String,
    /// Process objects to capture (app tap) or to leave out (global tap).
    pub processes: Vec<ObjId>,
    pub global: bool,
    pub out_uid: String,
    pub frames: u32,
    pub app: Option<Arc<SharedChain>>,
    pub dev: Option<Arc<SharedChain>>,
}

struct Unit {
    spec: TapSpec,
    desc: Retained<CATapDescription>,
    tap: ObjId,
    agg: ObjId,
    proc_id: AudioDeviceIOProcID,
    ctx: *mut IoContext,
    stats: Arc<TapStats>,
    frames: u32,
    sample_rate: f64,
    latency_frames: u32,
    escalate_seen: (u32, Instant),
}

fn same_arc(a: &Option<Arc<SharedChain>>, b: &Option<Arc<SharedChain>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

fn ns_numbers(ids: &[ObjId]) -> Retained<NSArray<NSNumber>> {
    let nums: Vec<Retained<NSNumber>> = ids.iter().map(|i| NSNumber::new_u32(*i)).collect();
    let refs: Vec<&NSNumber> = nums.iter().map(|n| &**n).collect();
    NSArray::from_slice(&refs)
}

fn make_description(spec: &TapSpec) -> Retained<CATapDescription> {
    unsafe {
        let arr = ns_numbers(&spec.processes);
        let d = if spec.global {
            CATapDescription::initStereoGlobalTapButExcludeProcesses(CATapDescription::alloc(), &arr)
        } else {
            CATapDescription::initStereoMixdownOfProcesses(CATapDescription::alloc(), &arr)
        };
        d.setName(&NSString::from_str(&format!("SoundMixer {}", spec.label)));
        d.setPrivate(true);
        d.setMuteBehavior(CATapMuteBehavior::Muted);
        d
    }
}

fn dict(pairs: &[(&str, &AnyObject)]) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let krefs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let vals: Vec<&AnyObject> = pairs.iter().map(|(_, v)| *v).collect();
    NSDictionary::from_slices(&krefs, &vals)
}

fn create_unit(spec: TapSpec) -> Result<Unit, String> {
    let out_id = ca::device_by_uid(&spec.out_uid).ok_or("output device is gone")?;
    let desc = make_description(&spec);
    let mut tap: ObjId = 0;
    let st = unsafe { AudioHardwareCreateProcessTap(Some(&desc), &mut tap) };
    if st != 0 || tap == 0 {
        return Err(format!(
            "macOS refused the audio tap (error {st}). Allow audio recording for this app in System Settings > Privacy & Security."
        ));
    }
    let tap_uuid = unsafe { desc.UUID().UUIDString().to_string() };

    // private aggregate: the output device is the clock; the tap is drift-compensated to it
    let one = NSNumber::new_u32(1);
    let zero = NSNumber::new_u32(0);
    let out_uid = NSString::from_str(&spec.out_uid);
    let tap_uid = NSString::from_str(&tap_uuid);
    let sub = dict(&[("uid", out_uid.as_ref()), ("drift", zero.as_ref())]);
    let subs = NSArray::from_slice(&[&*sub]);
    let tapd = dict(&[("uid", tap_uid.as_ref()), ("drift", one.as_ref())]);
    let taps = NSArray::from_slice(&[&*tapd]);
    let agg_uid = NSString::from_str(&format!("rs.soundmixer.agg.{}", tap_uuid));
    let agg_name = NSString::from_str(&format!("SoundMixer {}", spec.label));
    let desc_dict = dict(&[
        ("uid", agg_uid.as_ref()),
        ("name", agg_name.as_ref()),
        ("master", out_uid.as_ref()),
        ("clock", out_uid.as_ref()),
        ("private", one.as_ref()),
        ("stacked", zero.as_ref()),
        ("tapautostart", one.as_ref()),
        ("subdevices", subs.as_ref()),
        ("taps", taps.as_ref()),
    ]);
    let mut agg: ObjId = 0;
    let cf: &CFDictionary = unsafe { &*(Retained::as_ptr(&desc_dict) as *const CFDictionary) };
    let st = unsafe { AudioHardwareCreateAggregateDevice(cf, NonNull::from(&mut agg)) };
    if st != 0 || agg == 0 {
        unsafe { AudioHardwareDestroyProcessTap(tap) };
        return Err(format!("could not create the playback device (error {st})"));
    }

    ca::set_buffer_frames(agg, spec.frames);
    let frames = ca::buffer_frames(agg).max(1);
    let mut sr = ca::sample_rate(agg);
    if sr <= 0.0 {
        sr = ca::sample_rate(out_id).max(44100.0);
    }
    let (c1, c2) = ca::stereo_channels(out_id);
    let stats = Arc::new(TapStats::default());
    let mut app_chain = Chain::new(false);
    let mut dev_chain = Chain::new(false);
    let mut app_gen = 0;
    let mut dev_gen = 0;
    if let Some(a) = &spec.app {
        app_chain.configure(&a.get(), sr);
        app_gen = a.gen.load(Ordering::Acquire);
    }
    if let Some(d) = &spec.dev {
        dev_chain.configure(&d.get(), sr);
        dev_gen = d.gen.load(Ordering::Acquire);
    }
    let ctx = Box::into_raw(Box::new(IoContext {
        app: spec.app.clone(),
        dev: spec.dev.clone(),
        app_gen,
        dev_gen,
        app_chain,
        dev_chain,
        l: vec![0.0; MAX_FRAMES].into_boxed_slice(),
        r: vec![0.0; MAX_FRAMES].into_boxed_slice(),
        sr,
        stereo: ((c1 - 1) as usize, (c2 - 1) as usize),
        fade: 0.0,
        fade_step: 1.0 / (FADE_S * sr as f32).max(1.0),
        stats: stats.clone(),
    }));

    let mut proc_id: AudioDeviceIOProcID = None;
    let st = unsafe { AudioDeviceCreateIOProcID(agg, Some(io_proc), ctx as *mut c_void, NonNull::from(&mut proc_id)) };
    if st != 0 || proc_id.is_none() {
        unsafe {
            AudioHardwareDestroyAggregateDevice(agg);
            AudioHardwareDestroyProcessTap(tap);
            drop(Box::from_raw(ctx));
        }
        return Err(format!("could not start audio processing (error {st})"));
    }
    // SAFETY: `stats` is kept in the Unit and the listener is removed in destroy_unit before it drops
    unsafe {
        ca::add_listener(
            agg,
            &ca::addr(kAudioDeviceProcessorOverload, ca::SCOPE_GLOBAL, ca::ELEM_MAIN),
            overload_listener,
            Arc::as_ptr(&stats) as *mut c_void,
        )
    };
    let st = unsafe { AudioDeviceStart(agg, proc_id) };
    if st != 0 {
        let u = Unit {
            spec,
            desc,
            tap,
            agg,
            proc_id,
            ctx,
            stats,
            frames,
            sample_rate: sr,
            latency_frames: 0,
            escalate_seen: (0, Instant::now()),
        };
        destroy_unit(u);
        return Err(format!("the output device would not start (error {st})"));
    }
    let latency_frames = ca::output_latency_frames(out_id) + frames + dsp::LIMIT_LOOKAHEAD as u32;
    Ok(Unit { spec, desc, tap, agg, proc_id, ctx, stats, frames, sample_rate: sr, latency_frames, escalate_seen: (0, Instant::now()) })
}

fn destroy_unit(u: Unit) {
    unsafe {
        AudioDeviceStop(u.agg, u.proc_id);
        ca::remove_listener(
            u.agg,
            &ca::addr(kAudioDeviceProcessorOverload, ca::SCOPE_GLOBAL, ca::ELEM_MAIN),
            overload_listener,
            Arc::as_ptr(&u.stats) as *mut c_void,
        );
        AudioDeviceDestroyIOProcID(u.agg, u.proc_id);
        AudioHardwareDestroyAggregateDevice(u.agg);
        AudioHardwareDestroyProcessTap(u.tap);
        drop(Box::from_raw(u.ctx));
    }
}

/// Change which processes a running tap captures, without stopping it (no gap, no click).
fn retarget(u: &mut Unit, processes: &[ObjId]) -> bool {
    unsafe {
        u.desc.setProcesses(&ns_numbers(processes));
        let p: *const CATapDescription = &*u.desc;
        ca::set(u.tap, &ca::addr(kAudioTapPropertyDescription, ca::SCOPE_GLOBAL, ca::ELEM_MAIN), p)
    }
}

// ------------------------------------------------------------------ engine

pub struct TapReport {
    pub stats: Arc<TapStats>,
    pub frames: u32,
    pub sample_rate: f64,
    pub latency_ms: f64,
    pub escalated: bool,
}

pub struct Engine {
    units: HashMap<String, Unit>,
    retiring: Vec<(Unit, Instant)>,
    pub errors: HashMap<String, String>,
    watched_devices: Vec<ObjId>,
    listening: bool,
}

const SYSTEM_SELECTORS: [u32; 5] = [
    kAudioHardwarePropertyDevices,
    kAudioHardwarePropertyDefaultOutputDevice,
    kAudioHardwarePropertyDefaultInputDevice,
    kAudioHardwarePropertyDefaultSystemOutputDevice,
    kAudioHardwarePropertyProcessObjectList,
];

const DEVICE_SELECTORS: [u32; 4] = [
    kAudioDevicePropertyNominalSampleRate,
    kAudioDevicePropertyDeviceIsAlive,
    kAudioDevicePropertyStreamConfiguration,
    kAudioDevicePropertyPreferredChannelsForStereo,
];

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub fn new() -> Self {
        Self { units: HashMap::new(), retiring: Vec::new(), errors: HashMap::new(), watched_devices: Vec::new(), listening: false }
    }

    /// Listen for device, default-device and app changes (sets DIRTY).
    pub fn listen(&mut self, devices: &[ca::Device]) {
        if !self.listening {
            for sel in SYSTEM_SELECTORS {
                // SAFETY: no data pointer
                unsafe {
                    ca::add_listener(ca::SYSTEM, &ca::addr(sel, ca::SCOPE_GLOBAL, ca::ELEM_MAIN), dirty_listener, std::ptr::null_mut())
                };
            }
            self.listening = true;
        }
        let ids: Vec<ObjId> = devices.iter().filter(|d| d.out_channels > 0).map(|d| d.id).collect();
        for id in &ids {
            if !self.watched_devices.contains(id) {
                for sel in DEVICE_SELECTORS {
                    let scope = if sel == kAudioDevicePropertyStreamConfiguration || sel == kAudioDevicePropertyPreferredChannelsForStereo {
                        ca::SCOPE_OUT
                    } else {
                        ca::SCOPE_GLOBAL
                    };
                    // SAFETY: no data pointer
                    unsafe { ca::add_listener(*id, &ca::addr(sel, scope, ca::ELEM_MAIN), dirty_listener, std::ptr::null_mut()) };
                }
            }
        }
        self.watched_devices.retain(|id| ids.contains(id));
        for id in ids {
            if !self.watched_devices.contains(&id) {
                self.watched_devices.push(id);
            }
        }
    }

    pub fn report(&self, key: &str) -> Option<TapReport> {
        self.units.get(key).map(|u| TapReport {
            stats: u.stats.clone(),
            frames: u.frames,
            sample_rate: u.sample_rate,
            latency_ms: u.latency_frames as f64 * 1000.0 / u.sample_rate.max(1.0),
            escalated: u.frames > u.spec.frames,
        })
    }

    pub fn active_count(&self) -> usize {
        self.units.len()
    }

    /// Bring the running taps in line with `specs`. Device/format changes crossfade old -> new.
    pub fn reconcile(&mut self, specs: Vec<TapSpec>, force_rebuild: bool) {
        let now = Instant::now();
        // destroy taps whose fade-out finished
        let mut keep = Vec::new();
        for (u, at) in self.retiring.drain(..) {
            if now.duration_since(at) >= RETIRE_AFTER {
                destroy_unit(u);
            } else {
                keep.push((u, at));
            }
        }
        self.retiring = keep;

        let wanted: HashMap<String, TapSpec> = specs.into_iter().map(|s| (s.key.clone(), s)).collect();
        let stale: Vec<String> = self.units.keys().filter(|k| !wanted.contains_key(*k)).cloned().collect();
        for k in stale {
            if let Some(u) = self.units.remove(&k) {
                u.stats.fading_out.store(true, Ordering::Relaxed);
                self.retiring.push((u, now));
            }
            self.errors.remove(&k);
        }

        for (key, spec) in wanted {
            let needs_new = match self.units.get_mut(&key) {
                None => true,
                Some(u) => {
                    let same_route = u.spec.out_uid == spec.out_uid && u.spec.global == spec.global;
                    let same_chains = same_arc(&u.spec.app, &spec.app) && same_arc(&u.spec.dev, &spec.dev);
                    let sr_now = ca::device_by_uid(&spec.out_uid).map(ca::sample_rate).unwrap_or(u.sample_rate);
                    // retarget runs only when nothing else forces a rebuild (short-circuit)
                    if force_rebuild
                        || !same_route
                        || !same_chains
                        || (sr_now > 0.0 && sr_now != u.sample_rate)
                        || (u.spec.processes != spec.processes && !retarget(u, &spec.processes))
                    {
                        true
                    } else {
                        u.spec.processes = spec.processes.clone();
                        if u.spec.frames != spec.frames {
                            ca::set_buffer_frames(u.agg, spec.frames);
                            u.frames = ca::buffer_frames(u.agg).max(1);
                            u.spec.frames = spec.frames;
                        }
                        false
                    }
                }
            };
            if needs_new {
                self.replace(key, spec);
            }
        }
    }

    /// Only finish off taps that have faded out.
    pub fn reconcile_retiring_only(&mut self) {
        let now = Instant::now();
        let mut keep = Vec::new();
        for (u, at) in self.retiring.drain(..) {
            if now.duration_since(at) >= RETIRE_AFTER {
                destroy_unit(u);
            } else {
                keep.push((u, at));
            }
        }
        self.retiring = keep;
    }

    fn replace(&mut self, key: String, spec: TapSpec) {
        // start the new tap first: it fades in while the old one fades out (no gap, no blast)
        match create_unit(spec) {
            Ok(nu) => {
                if let Some(old) = self.units.insert(key.clone(), nu) {
                    old.stats.fading_out.store(true, Ordering::Relaxed);
                    self.retiring.push((old, Instant::now()));
                }
                self.errors.remove(&key);
            }
            Err(e) => {
                if let Some(old) = self.units.remove(&key) {
                    old.stats.fading_out.store(true, Ordering::Relaxed);
                    self.retiring.push((old, Instant::now()));
                }
                self.errors.insert(key, e);
            }
        }
    }

    /// Grow the buffer of any tap that keeps running late or overloading (a self-tuning
    /// switch to "buffered mode"). Returns true if anything changed.
    pub fn watchdog(&mut self) -> bool {
        let mut changed = false;
        for u in self.units.values_mut() {
            let trouble = u.stats.late.load(Ordering::Relaxed) + u.stats.overloads.load(Ordering::Relaxed);
            let (seen, since) = u.escalate_seen;
            if since.elapsed() > ESCALATE_WINDOW {
                u.escalate_seen = (trouble, Instant::now());
                continue;
            }
            if trouble.saturating_sub(seen) >= ESCALATE_AFTER && u.frames < MAX_ESCALATED_FRAMES {
                let next = (u.frames * 2).min(MAX_ESCALATED_FRAMES);
                ca::set_buffer_frames(u.agg, next);
                u.frames = ca::buffer_frames(u.agg).max(next);
                u.escalate_seen = (trouble, Instant::now());
                changed = true;
            }
        }
        changed
    }

    pub fn shutdown(&mut self) {
        for (_, u) in self.units.drain() {
            destroy_unit(u);
        }
        for (u, _) in self.retiring.drain(..) {
            destroy_unit(u);
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}
