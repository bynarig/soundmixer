//! Real-time audio processing. Everything in `process` paths is allocation-free and lock-free.
//! Every tuning constant is documented next to its definition below.

use serde::{Deserialize, Serialize};

/// Largest IO buffer the engine processes in one call (frames). Bigger calls are split.
pub const MAX_FRAMES: usize = 4096;

/// 10-band graphic EQ centre frequencies (Hz).
pub const EQ_FREQS: [f64; 10] = [32.0, 64.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// Q of each band-pass filter in the graphic EQ.
pub const EQ_Q: f64 = 2.8;
/// Slider value +/-1 maps to +/-12 dB.
pub const EQ_RANGE_DB: f32 = 12.0;
/// Weight applied to each band's (linear gain - 1) before it is added to the dry signal.
pub const EQ_BAND_WEIGHT: f64 = 0.25;

/// Volume smoother time (seconds) and snap threshold.
pub const SMOOTH_SPEED_S: f32 = 0.01;
pub const SMOOTH_SNAP: f32 = 1e-4;

/// Peak limiter: lookahead/attack frames at <= 64 kHz (doubled for every 2x above),
/// release to 10 % within 25 ms, output ceiling.
pub const LIMIT_LOOKAHEAD: usize = 64;
pub const LIMIT_ATTACK: usize = 12;
pub const LIMIT_RELEASE_S: f32 = 0.025;
pub const LIMIT_RELEASE_TARGET: f32 = 0.1;
pub const LIMIT_CEILING: f32 = 0.995;
const LIMIT_ATTACK_LN: f64 = -4.615121461073711;

/// Boost: soft compressor followed by make-up gain, then the limiter catches peaks.
pub const BOOST_THRESHOLD_DB: f32 = -24.0;
pub const BOOST_RATIO: f32 = 3.0;
pub const BOOST_MAKEUP_DB: f32 = 9.0;
pub const BOOST_ATTACK_S: f32 = 0.005;
pub const BOOST_RELEASE_S: f32 = 0.150;

/// Fade used when a tap starts, stops or is swapped (seconds, estimated).
pub const FADE_S: f32 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Eq10Settings {
    pub enabled: bool,
    /// -1..1 (maps to +/-12 dB)
    pub preamp: f32,
    /// -1..1 each (maps to +/-12 dB)
    pub bands: [f32; 10],
}

impl Default for Eq10Settings {
    fn default() -> Self {
        Self { enabled: false, preamp: 0.0, bands: [0.0; 10] }
    }
}

impl Eq10Settings {
    pub fn is_neutral(&self) -> bool {
        !self.enabled || (self.preamp == 0.0 && self.bands.iter().all(|b| *b == 0.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterKind {
    Peak,
    LowShelf,
    HighShelf,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeqBand {
    pub kind: FilterKind,
    pub freq: f32,
    pub gain_db: f32,
    pub q: f32,
}

pub const PEQ_MAX: usize = 16;

/// Parametric EQ used for headphone correction profiles.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeqSettings {
    pub enabled: bool,
    pub preamp_db: f32,
    pub count: usize,
    pub bands: [PeqBand; PEQ_MAX],
}

impl Default for PeqSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp_db: 0.0,
            count: 0,
            bands: [PeqBand { kind: FilterKind::Peak, freq: 1000.0, gain_db: 0.0, q: 1.0 }; PEQ_MAX],
        }
    }
}

impl PeqSettings {
    pub fn is_neutral(&self) -> bool {
        !self.enabled || (self.preamp_db == 0.0 && self.bands[..self.count].iter().all(|b| b.gain_db == 0.0))
    }
}

/// Everything one processing chain needs. Plain data, copied into the audio thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChainSettings {
    /// 0..MAX_APP_VOLUME (1.0 = unchanged)
    pub volume: f32,
    pub muted: bool,
    pub boost: bool,
    /// -1 (left) .. 1 (right)
    pub balance: f32,
    pub eq: Eq10Settings,
    pub peq: PeqSettings,
}

impl Default for ChainSettings {
    fn default() -> Self {
        Self { volume: 1.0, muted: false, boost: false, balance: 0.0, eq: Eq10Settings::default(), peq: PeqSettings::default() }
    }
}

impl ChainSettings {
    pub fn is_neutral(&self) -> bool {
        self.volume == 1.0 && !self.muted && !self.boost && self.balance == 0.0 && self.eq.is_neutral() && self.peq.is_neutral()
    }
}

#[inline]
fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

// ---------------------------------------------------------------- biquad

#[derive(Clone, Copy, Default)]
struct Coefs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

#[derive(Clone, Copy, Default)]
struct State {
    s1: f64,
    s2: f64,
}

impl Coefs {
    #[inline]
    fn run(&self, st: &mut State, x: f64) -> f64 {
        let y = self.b0 * x + st.s1;
        st.s1 = self.b1 * x - self.a1 * y + st.s2;
        st.s2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Constant 0 dB peak gain band-pass (as used by the graphic EQ).
    fn bandpass(fc: f64, q: f64, sr: f64) -> Option<Coefs> {
        if fc >= sr * 0.5 {
            return None;
        }
        let w = std::f64::consts::TAU * fc / sr;
        let (s, c) = w.sin_cos();
        let alpha = s / q;
        let a0 = 1.0 + alpha;
        Some(Coefs { b0: alpha / a0, b1: 0.0, b2: -alpha / a0, a1: -2.0 * c / a0, a2: (1.0 - alpha) / a0 })
    }

    fn peq(band: &PeqBand, sr: f64) -> Option<Coefs> {
        let fc = band.freq as f64;
        if fc.is_nan() || fc <= 0.0 || fc >= sr * 0.5 || band.gain_db == 0.0 {
            return None;
        }
        let q = (band.q as f64).max(0.05);
        let a = 10f64.powf(band.gain_db as f64 / 40.0);
        let w = std::f64::consts::TAU * fc / sr;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        let (b0, b1, b2, a0, a1, a2) = match band.kind {
            FilterKind::Peak => (1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a),
            FilterKind::LowShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * c + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                    a * ((a + 1.0) - (a - 1.0) * c - k),
                    (a + 1.0) + (a - 1.0) * c + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * c),
                    (a + 1.0) + (a - 1.0) * c - k,
                )
            }
            FilterKind::HighShelf => {
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * c + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                    a * ((a + 1.0) + (a - 1.0) * c - k),
                    (a + 1.0) - (a - 1.0) * c + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * c),
                    (a + 1.0) - (a - 1.0) * c - k,
                )
            }
        };
        Some(Coefs { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 })
    }
}

// ---------------------------------------------------------------- graphic EQ

struct GraphicEq {
    active: bool,
    preamp: f64,
    coefs: [Option<Coefs>; 10],
    weights: [f64; 10],
    state: [[State; 10]; 2],
}

impl GraphicEq {
    fn new() -> Self {
        Self { active: false, preamp: 1.0, coefs: [None; 10], weights: [0.0; 10], state: [[State::default(); 10]; 2] }
    }

    fn configure(&mut self, s: &Eq10Settings, sr: f64) {
        let was = self.active;
        self.active = !s.is_neutral();
        self.preamp = db_to_lin(s.preamp.clamp(-1.0, 1.0) * EQ_RANGE_DB) as f64;
        for (i, &fc) in EQ_FREQS.iter().enumerate() {
            self.coefs[i] = Coefs::bandpass(fc, EQ_Q, sr);
            let lin = db_to_lin(s.bands[i].clamp(-1.0, 1.0) * EQ_RANGE_DB) as f64;
            self.weights[i] = (lin - 1.0) * EQ_BAND_WEIGHT;
        }
        if !was && self.active {
            self.reset();
        }
    }

    fn reset(&mut self) {
        self.state = [[State::default(); 10]; 2];
    }

    #[inline]
    fn process(&mut self, ch: usize, buf: &mut [f32]) {
        if !self.active {
            return;
        }
        let st = &mut self.state[ch];
        for x in buf.iter_mut() {
            let dry = *x as f64 * self.preamp;
            let mut y = dry;
            // every band keeps running so moving a slider away from 0 never starts from stale state
            for ((c, w), st) in self.coefs.iter().zip(&self.weights).zip(st.iter_mut()) {
                if let Some(c) = c {
                    y += w * c.run(st, dry);
                }
            }
            *x = y as f32;
        }
    }
}

// ---------------------------------------------------------------- parametric EQ

struct ParamEq {
    active: bool,
    preamp: f32,
    count: usize,
    coefs: [Option<Coefs>; PEQ_MAX],
    state: [[State; PEQ_MAX]; 2],
}

impl ParamEq {
    fn new() -> Self {
        Self { active: false, preamp: 1.0, count: 0, coefs: [None; PEQ_MAX], state: [[State::default(); PEQ_MAX]; 2] }
    }

    fn configure(&mut self, s: &PeqSettings, sr: f64) {
        let was = self.active;
        self.active = !s.is_neutral();
        self.preamp = db_to_lin(s.preamp_db.clamp(-24.0, 12.0));
        self.count = s.count.min(PEQ_MAX);
        for i in 0..PEQ_MAX {
            self.coefs[i] = if i < self.count { Coefs::peq(&s.bands[i], sr) } else { None };
        }
        if !was && self.active {
            self.state = [[State::default(); PEQ_MAX]; 2];
        }
    }

    #[inline]
    fn process(&mut self, ch: usize, buf: &mut [f32]) {
        if !self.active {
            return;
        }
        let st = &mut self.state[ch];
        for x in buf.iter_mut() {
            let mut y = *x as f64 * self.preamp as f64;
            for (c, st) in self.coefs[..self.count].iter().zip(st.iter_mut()) {
                if let Some(c) = c {
                    y = c.run(st, y);
                }
            }
            *x = y as f32;
        }
    }
}

// ---------------------------------------------------------------- magic boost

struct Booster {
    on: bool,
    env_db: f32,
    att: f32,
    rel: f32,
    makeup: f32,
}

impl Booster {
    fn new() -> Self {
        Self { on: false, env_db: -120.0, att: 0.0, rel: 0.0, makeup: 1.0 }
    }
    fn configure(&mut self, on: bool, sr: f64) {
        self.on = on;
        self.att = (-1.0 / (BOOST_ATTACK_S * sr as f32)).exp();
        self.rel = (-1.0 / (BOOST_RELEASE_S * sr as f32)).exp();
        self.makeup = db_to_lin(BOOST_MAKEUP_DB);
    }
    #[inline]
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        if !self.on {
            return;
        }
        for i in 0..l.len() {
            let peak = l[i].abs().max(r[i].abs()).max(1e-6);
            let lvl = 20.0 * peak.log10();
            let c = if lvl > self.env_db { self.att } else { self.rel };
            self.env_db = lvl + c * (self.env_db - lvl);
            let over = self.env_db - BOOST_THRESHOLD_DB;
            let gr_db = if over > 0.0 { over / BOOST_RATIO - over } else { 0.0 };
            let g = db_to_lin(gr_db) * self.makeup;
            l[i] *= g;
            r[i] *= g;
        }
    }
}

// ---------------------------------------------------------------- volume smoother

/// One-pole per-channel smoother: y = y*a + target*(1-a), a = exp(-(pi/2)/(speed*sr)).
struct Smoother {
    a: f32,
    b: f32,
    cur: [f32; 2],
    target: f32,
}

impl Smoother {
    fn new(initial: f32) -> Self {
        let mut s = Self { a: 0.0, b: 1.0, cur: [initial; 2], target: initial };
        s.set_rate(44100.0, SMOOTH_SPEED_S);
        s
    }
    fn set_rate(&mut self, sr: f64, speed: f32) {
        let speed = speed.max(0.001);
        self.a = (-(std::f32::consts::FRAC_PI_2) / (speed * sr as f32)).exp();
        self.b = 1.0 - self.a;
    }
    #[inline]
    fn process(&mut self, ch: usize, buf: &mut [f32]) {
        let t = self.target;
        let mut c = self.cur[ch];
        if (c - t).abs() < SMOOTH_SNAP {
            c = t;
            if t != 1.0 {
                for x in buf.iter_mut() {
                    *x *= t;
                }
            }
        } else {
            for x in buf.iter_mut() {
                c = c * self.a + t * self.b;
                if (c - t).abs() < SMOOTH_SNAP {
                    c = t;
                }
                *x *= c;
            }
        }
        self.cur[ch] = c;
    }
}

// ---------------------------------------------------------------- lookahead peak limiter

const LIMIT_MAX_LA: usize = LIMIT_LOOKAHEAD * 4;

struct Limiter {
    la: usize,
    delay: [[f32; LIMIT_MAX_LA]; 2],
    peaks: [f32; LIMIT_MAX_LA],
    pos: usize,
    win_max: f32,
    gain: f32,
    attack: f32,
    release: f32,
}

impl Limiter {
    fn new() -> Self {
        Self {
            la: LIMIT_LOOKAHEAD,
            delay: [[0.0; LIMIT_MAX_LA]; 2],
            peaks: [0.0; LIMIT_MAX_LA],
            pos: 0,
            win_max: 0.0,
            gain: 1.0,
            attack: 0.5,
            release: 0.001,
        }
    }

    fn configure(&mut self, sr: f64) {
        let mut la = LIMIT_LOOKAHEAD;
        let mut att = LIMIT_ATTACK;
        let mut r = sr;
        while r > 64000.0 && la * 2 <= LIMIT_MAX_LA {
            la *= 2;
            att *= 2;
            r *= 0.5;
        }
        if la != self.la {
            self.la = la;
            self.reset();
        }
        self.attack = (1.0 - (LIMIT_ATTACK_LN / (att as f64 + 1.0)).exp()) as f32;
        let n = (sr as f32 * LIMIT_RELEASE_S).max(1.0);
        self.release = 1.0 - 10f32.powf(LIMIT_RELEASE_TARGET.log10() / n);
    }

    fn reset(&mut self) {
        self.delay = [[0.0; LIMIT_MAX_LA]; 2];
        self.peaks = [0.0; LIMIT_MAX_LA];
        self.pos = 0;
        self.win_max = 0.0;
        self.gain = 1.0;
    }

    #[inline]
    fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let la = self.la;
        for i in 0..l.len() {
            let p = l[i].abs().max(r[i].abs());
            let outgoing = self.peaks[self.pos];
            self.peaks[self.pos] = p;
            if p >= self.win_max {
                self.win_max = p;
            } else if outgoing >= self.win_max {
                let mut m = 0.0f32;
                for v in &self.peaks[..la] {
                    m = m.max(*v);
                }
                self.win_max = m;
            }
            let target = if self.win_max > LIMIT_CEILING { LIMIT_CEILING / self.win_max } else { 1.0 };
            let coef = if target < self.gain { self.attack } else { self.release };
            self.gain += (target - self.gain) * coef;
            let dl = self.delay[0][self.pos];
            let dr = self.delay[1][self.pos];
            self.delay[0][self.pos] = l[i];
            self.delay[1][self.pos] = r[i];
            self.pos += 1;
            if self.pos >= la {
                self.pos = 0;
            }
            // hard safety clip after the limiter (only reached on attack residue)
            l[i] = (dl * self.gain).clamp(-1.0, 1.0);
            r[i] = (dr * self.gain).clamp(-1.0, 1.0);
        }
    }
}

// ---------------------------------------------------------------- full chain

pub struct Chain {
    sr: f64,
    settings: ChainSettings,
    eq: GraphicEq,
    peq: ParamEq,
    boost: Booster,
    smoother: Smoother,
    limiter: Limiter,
}

impl Chain {
    pub fn new(start_silent: bool) -> Self {
        let mut c = Self {
            sr: 48000.0,
            settings: ChainSettings::default(),
            eq: GraphicEq::new(),
            peq: ParamEq::new(),
            boost: Booster::new(),
            smoother: Smoother::new(if start_silent { 0.0 } else { 1.0 }),
            limiter: Limiter::new(),
        };
        c.configure(&ChainSettings::default(), 48000.0);
        c
    }

    /// Apply new settings. Allocation-free; safe to call on the audio thread.
    pub fn configure(&mut self, s: &ChainSettings, sr: f64) {
        let sr_changed = sr != self.sr;
        self.sr = sr;
        self.settings = *s;
        self.eq.configure(&s.eq, sr);
        self.peq.configure(&s.peq, sr);
        self.boost.configure(s.boost, sr);
        self.smoother.set_rate(sr, SMOOTH_SPEED_S);
        self.smoother.target = if s.muted { 0.0 } else { s.volume.max(0.0) };
        if sr_changed {
            self.limiter.configure(sr);
            self.reset();
        } else {
            self.limiter.configure(sr);
        }
    }

    pub fn reset(&mut self) {
        self.eq.reset();
        self.peq.state = [[State::default(); PEQ_MAX]; 2];
        self.limiter.reset();
    }

    /// Process one planar stereo block in place.
    #[inline]
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.eq.process(0, l);
        self.eq.process(1, r);
        self.peq.process(0, l);
        self.peq.process(1, r);
        self.boost.process(l, r);
        let b = self.settings.balance.clamp(-1.0, 1.0);
        if b < 0.0 {
            let g = 1.0 + b;
            r.iter_mut().for_each(|x| *x *= g);
        } else if b > 0.0 {
            let g = 1.0 - b;
            l.iter_mut().for_each(|x| *x *= g);
        }
        self.smoother.process(0, l);
        self.smoother.process(1, r);
        self.limiter.process(l, r);
        // Stability guard: a non-finite result resets every filter and
        // outputs silence for this block instead of a burst of noise.
        let mut acc = 0.0f32;
        for i in 0..l.len() {
            acc += l[i] + r[i];
        }
        if !acc.is_finite() {
            self.reset();
            l.iter_mut().for_each(|x| *x = 0.0);
            r.iter_mut().for_each(|x| *x = 0.0);
        }
    }
}

/// Peak level of a block (for the meters).
#[inline]
pub fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |m, x| m.max(x.abs()))
}

/// A filter from a profile is used only if all its numbers are finite and the frequency and Q are
/// positive; anything else would make the filter unstable (and the chain's guard would silence it).
fn valid_band(fc: f32, gain: f32, q: f32) -> bool {
    fc.is_finite() && fc > 0.0 && gain.is_finite() && q.is_finite() && q > 0.0
}

/// Parse a headphone EQ profile: JSON ({"name","preamp","bands":[{"type","fc","gain","bw","on"}]})
/// or AutoEQ's ParametricEQ.txt ("Preamp: -6 dB" / "Filter 1: ON PK Fc 100 Hz Gain 2 dB Q 0.7").
pub fn parse_profile(text: &str) -> Result<(String, PeqSettings), String> {
    fn num(v: &serde_json::Value) -> Option<f32> {
        match v {
            serde_json::Value::Number(n) => n.as_f64().map(|f| f as f32),
            serde_json::Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }
    fn kind(s: &str) -> Option<FilterKind> {
        match s.trim().to_ascii_uppercase().as_str() {
            "PK" | "PEQ" | "PEAK" => Some(FilterKind::Peak),
            "LSC" | "LS" | "LOWSHELF" | "LOW_SHELF" => Some(FilterKind::LowShelf),
            "HSC" | "HS" | "HIGHSHELF" | "HIGH_SHELF" => Some(FilterKind::HighShelf),
            _ => None,
        }
    }
    let mut p = PeqSettings { enabled: true, ..Default::default() };
    let t = text.trim();
    if t.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(t).map_err(|e| format!("Not valid JSON: {e}"))?;
        let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("Custom profile").to_string();
        p.preamp_db = v.get("preamp").and_then(num).filter(|v| v.is_finite()).unwrap_or(0.0);
        if let Some(bands) = v.get("bands").and_then(|b| b.as_array()) {
            for b in bands {
                if p.count >= PEQ_MAX {
                    break;
                }
                if b.get("on").and_then(|o| o.as_bool()) == Some(false) {
                    continue;
                }
                let k = b.get("type").and_then(|t| t.as_str()).and_then(kind).unwrap_or(FilterKind::Peak);
                let (Some(fc), Some(g)) = (b.get("fc").and_then(num), b.get("gain").and_then(num)) else { continue };
                let q = b.get("bw").or_else(|| b.get("q")).and_then(num).unwrap_or(0.707);
                if !valid_band(fc, g, q) {
                    continue;
                }
                p.bands[p.count] = PeqBand { kind: k, freq: fc, gain_db: g, q };
                p.count += 1;
            }
        }
        if p.count == 0 {
            return Err("The profile has no filters.".into());
        }
        return Ok((name, p));
    }
    for line in t.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        if words[0].eq_ignore_ascii_case("preamp:") {
            if let Some(v) = words.get(1).and_then(|w| w.parse::<f32>().ok()).filter(|v| v.is_finite()) {
                p.preamp_db = v;
            }
            continue;
        }
        if !words[0].eq_ignore_ascii_case("filter") && !words[0].to_ascii_lowercase().starts_with("filter") {
            continue;
        }
        if words.iter().any(|w| w.eq_ignore_ascii_case("OFF")) || p.count >= PEQ_MAX {
            continue;
        }
        let k = words.iter().find_map(|w| kind(w));
        let after = |key: &str| -> Option<f32> {
            words.iter().position(|w| w.eq_ignore_ascii_case(key)).and_then(|i| words.get(i + 1)).and_then(|w| w.parse().ok())
        };
        if let (Some(k), Some(fc), Some(g)) = (k, after("Fc"), after("Gain")) {
            let q = after("Q").unwrap_or(0.707);
            if valid_band(fc, g, q) {
                p.bands[p.count] = PeqBand { kind: k, freq: fc, gain_db: g, q };
                p.count += 1;
            }
        }
    }
    if p.count == 0 {
        return Err("No filters found. Paste a ParametricEQ.txt or a JSON profile.".into());
    }
    Ok(("Custom profile".into(), p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_chain_is_transparent_after_delay() {
        let mut c = Chain::new(false);
        let mut l = vec![0.5f32; 512];
        let mut r = vec![-0.25f32; 512];
        c.process(&mut l, &mut r);
        assert!((l[511] - 0.5).abs() < 1e-5);
        assert!((r[511] + 0.25).abs() < 1e-5);
    }

    #[test]
    fn limiter_never_exceeds_ceiling() {
        let mut c = Chain::new(false);
        let s = ChainSettings { volume: 4.0, ..Default::default() };
        c.configure(&s, 48000.0);
        let mut worst = 0.0f32;
        for block in 0..50 {
            let mut l: Vec<f32> = (0..256).map(|i| ((block * 256 + i) as f32 * 0.05).sin() * 0.9).collect();
            let mut r = l.clone();
            c.process(&mut l, &mut r);
            worst = worst.max(peak(&l)).max(peak(&r));
        }
        assert!(worst <= 1.0);
    }

    #[test]
    fn parses_autoeq_text() {
        let (_, p) = parse_profile(
            "Preamp: -6.2 dB\nFilter 1: ON PK Fc 105 Hz Gain -0.4 dB Q 0.70\nFilter 2: ON HSC Fc 10000 Hz Gain -1.3 dB Q 0.70",
        )
        .unwrap();
        assert_eq!(p.count, 2);
        assert_eq!(p.bands[1].kind, FilterKind::HighShelf);
    }

    #[test]
    fn profile_rejects_empty_and_garbage() {
        for t in [
            "",
            "   ",
            "\n\n",
            "hello world",
            "Preamp: -6 dB",
            "{}",
            "{\"bands\": []}",
            "{\"bands\": \"x\"}",
            "Filter 1: ON PK Fc Hz Gain dB",
        ] {
            assert!(parse_profile(t).is_err(), "{t:?}");
        }
        let e = parse_profile("{not json").unwrap_err();
        assert!(e.starts_with("Not valid JSON"), "{e}");
    }

    #[test]
    fn profile_skips_non_finite_and_invalid_filters() {
        let t = "Preamp: NaN dB\n\
                 Filter 1: ON PK Fc nan Hz Gain 1 dB Q 1\n\
                 Filter 2: ON PK Fc 100 Hz Gain inf dB Q 1\n\
                 Filter 3: ON PK Fc 100 Hz Gain 1 dB Q 0\n\
                 Filter 4: ON PK Fc -100 Hz Gain 1 dB Q 1\n\
                 Filter 5: ON PK Fc 100 Hz Gain 1 dB Q -1\n\
                 Filter 6: ON PK Fc 250 Hz Gain -2 dB Q 1.1";
        let (_, p) = parse_profile(t).unwrap();
        assert_eq!(p.count, 1, "only the valid filter is kept");
        assert_eq!(p.bands[0].freq, 250.0);
        assert_eq!(p.preamp_db, 0.0, "NaN preamp ignored");
        let j = r#"{"preamp":"NaN","bands":[{"type":"PK","fc":"inf","gain":1},{"type":"PK","fc":100,"gain":"-inf"},{"type":"PK","fc":100,"gain":2,"q":0},{"type":"PK","fc":100,"gain":2}]}"#;
        let (_, p) = parse_profile(j).unwrap();
        assert_eq!(p.count, 1);
        assert_eq!(p.bands[0].q, 0.707, "missing Q defaults");
        assert_eq!(p.preamp_db, 0.0);
    }

    #[test]
    fn profile_truncates_at_peq_max() {
        let t: String = (1..=40).map(|i| format!("Filter {i}: ON PK Fc {} Hz Gain 1 dB Q 1\n", i * 100)).collect();
        let (_, p) = parse_profile(&t).unwrap();
        assert_eq!(p.count, PEQ_MAX);
        assert_eq!(p.bands[PEQ_MAX - 1].freq, PEQ_MAX as f32 * 100.0, "first 16 kept in order");
        let bands: Vec<String> = (1..=40).map(|i| format!("{{\"type\":\"PK\",\"fc\":{},\"gain\":1}}", i * 10)).collect();
        let (_, p) = parse_profile(&format!("{{\"bands\":[{}]}}", bands.join(","))).unwrap();
        assert_eq!(p.count, PEQ_MAX);
    }

    #[test]
    fn profile_honours_off_and_case_and_crlf() {
        let t = "preamp: -3 dB\r\nfilter 1: off PK Fc 100 Hz Gain 1 dB Q 1\r\nFILTER 2: ON lsc fc 105 hz gain 4 db q 0.7\r\n";
        let (name, p) = parse_profile(t).unwrap();
        assert_eq!(name, "Custom profile");
        assert_eq!(p.count, 1);
        assert_eq!(p.bands[0].kind, FilterKind::LowShelf);
        assert_eq!(p.preamp_db, -3.0);
        let j = r#"{"name":"HD 600","preamp":"-6.5","bands":[{"on":false,"type":"PK","fc":100,"gain":1},{"type":"HSC","fc":"9000","gain":"-2","bw":"0.7"},{"type":"weird","fc":50,"gain":1}]}"#;
        let (name, p) = parse_profile(j).unwrap();
        assert_eq!(name, "HD 600");
        assert_eq!(p.preamp_db, -6.5);
        assert_eq!(p.count, 2);
        assert_eq!(p.bands[0].kind, FilterKind::HighShelf);
        assert_eq!(p.bands[1].kind, FilterKind::Peak, "unknown type falls back to peak");
        assert!(p.enabled);
    }

    #[test]
    fn huge_profile_text_is_fine() {
        let t = "x".repeat(2_000_000);
        assert!(parse_profile(&t).is_err());
        let t = format!("{}\nFilter 1: ON PK Fc 100 Hz Gain 1 dB Q 1", "garbage line\n".repeat(100_000));
        assert_eq!(parse_profile(&t).unwrap().1.count, 1);
    }

    #[test]
    fn neutral_settings_detection() {
        assert!(ChainSettings::default().is_neutral());
        let mut eq = Eq10Settings { enabled: false, preamp: 1.0, bands: [1.0; 10] };
        assert!(eq.is_neutral(), "disabled EQ is neutral whatever its bands");
        eq.enabled = true;
        assert!(!eq.is_neutral());
        let mut peq = PeqSettings { enabled: true, count: 0, ..Default::default() };
        assert!(peq.is_neutral());
        peq.bands[5].gain_db = 3.0;
        assert!(peq.is_neutral(), "bands beyond count are ignored");
        peq.count = PEQ_MAX;
        assert!(!peq.is_neutral());
        for v in [0.0, 0.999, 1.001, 4.0] {
            assert!(!ChainSettings { volume: v, ..Default::default() }.is_neutral());
        }
    }

    #[test]
    fn chain_survives_extreme_input() {
        let mut c = Chain::new(false);
        let s = ChainSettings {
            volume: 4.0,
            boost: true,
            balance: 1.0,
            eq: Eq10Settings { enabled: true, preamp: 1.0, bands: [1.0; 10] },
            ..Default::default()
        };
        c.configure(&s, 48000.0);
        for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1e30, -1e30, f32::MIN_POSITIVE] {
            let mut l = vec![x; 256];
            let mut r = vec![x; 256];
            c.process(&mut l, &mut r);
            assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() <= 1.0), "input {x}");
        }
        // and it recovers with normal audio afterwards
        let mut l: Vec<f32> = (0..2048).map(|i| (i as f32 * 0.03).sin() * 0.5).collect();
        let mut r = l.clone();
        c.process(&mut l, &mut r);
        assert!(l.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn chain_handles_empty_and_odd_buffers() {
        let mut c = Chain::new(false);
        c.configure(&ChainSettings::default(), 44100.0);
        c.process(&mut [], &mut []);
        let mut l = vec![0.1; MAX_FRAMES * 3 + 7];
        let mut r = vec![0.1; MAX_FRAMES * 3 + 7];
        c.process(&mut l, &mut r);
        assert!(l.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn chain_configures_at_extreme_sample_rates() {
        let s = ChainSettings { volume: 2.0, eq: Eq10Settings { enabled: true, preamp: 0.0, bands: [1.0; 10] }, ..Default::default() };
        for sr in [8000.0, 22050.0, 44100.0, 192000.0, 384000.0] {
            let mut c = Chain::new(false);
            c.configure(&s, sr);
            let mut l: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.1).sin()).collect();
            let mut r = l.clone();
            c.process(&mut l, &mut r);
            assert!(l.iter().chain(r.iter()).all(|v| v.is_finite() && v.abs() <= 1.0), "sr {sr}");
        }
    }

    #[test]
    fn peak_edges() {
        assert_eq!(peak(&[]), 0.0);
        assert_eq!(peak(&[-0.7, 0.2]), 0.7);
        assert_eq!(peak(&[f32::NEG_INFINITY]), f32::INFINITY);
    }
}
