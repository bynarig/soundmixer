//! Thin, safe-ish helpers over the Core Audio HAL (macOS).

#![allow(non_upper_case_globals)]

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2_core_audio::*;
use objc2_foundation::NSString;

pub type ObjId = AudioObjectID;
pub const SYSTEM: ObjId = kAudioObjectSystemObject as ObjId;

pub const SCOPE_GLOBAL: u32 = kAudioObjectPropertyScopeGlobal;
pub const SCOPE_OUT: u32 = kAudioObjectPropertyScopeOutput;
pub const SCOPE_IN: u32 = kAudioObjectPropertyScopeInput;
pub const ELEM_MAIN: u32 = kAudioObjectPropertyElementMain;

// AudioToolbox "virtual main volume/balance" (works on devices without a main volume control).
const VIRTUAL_MAIN_VOLUME: u32 = u32::from_be_bytes(*b"vmvc");
const VIRTUAL_MAIN_BALANCE: u32 = u32::from_be_bytes(*b"vmbl");

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioHardwareServiceHasProperty(id: ObjId, addr: *const AudioObjectPropertyAddress) -> u8;
    fn AudioHardwareServiceIsPropertySettable(id: ObjId, addr: *const AudioObjectPropertyAddress, out: *mut u8) -> i32;
    fn AudioHardwareServiceGetPropertyData(
        id: ObjId,
        addr: *const AudioObjectPropertyAddress,
        qsize: u32,
        q: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
    fn AudioHardwareServiceSetPropertyData(
        id: ObjId,
        addr: *const AudioObjectPropertyAddress,
        qsize: u32,
        q: *const c_void,
        size: u32,
        data: *const c_void,
    ) -> i32;
}

extern "C" {
    fn responsibility_get_pid_responsible_for_pid(pid: i32) -> i32;
    fn proc_name(pid: i32, buffer: *mut c_void, size: u32) -> i32;
}

pub fn addr(sel: u32, scope: u32, elem: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: sel, mScope: scope, mElement: elem }
}

pub fn has(id: ObjId, a: &AudioObjectPropertyAddress) -> bool {
    unsafe { AudioObjectHasProperty(id, NonNull::from(a)) }
}

pub fn settable(id: ObjId, a: &AudioObjectPropertyAddress) -> bool {
    let mut s: u8 = 0;
    let st = unsafe { AudioObjectIsPropertySettable(id, NonNull::from(a), NonNull::from(&mut s).cast()) };
    st == 0 && s != 0
}

pub fn get<T: Copy>(id: ObjId, a: &AudioObjectPropertyAddress) -> Option<T> {
    let mut v = std::mem::MaybeUninit::<T>::zeroed();
    let mut size = size_of::<T>() as u32;
    let st = unsafe {
        AudioObjectGetPropertyData(
            id,
            NonNull::from(a),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(v.as_mut_ptr() as *mut c_void).unwrap(),
        )
    };
    if st == 0 {
        Some(unsafe { v.assume_init() })
    } else {
        None
    }
}

pub fn get_with_qualifier<T: Copy, Q>(id: ObjId, a: &AudioObjectPropertyAddress, q: &Q) -> Option<T> {
    let mut v = std::mem::MaybeUninit::<T>::zeroed();
    let mut size = size_of::<T>() as u32;
    let st = unsafe {
        AudioObjectGetPropertyData(
            id,
            NonNull::from(a),
            size_of::<Q>() as u32,
            q as *const Q as *const c_void,
            NonNull::from(&mut size),
            NonNull::new(v.as_mut_ptr() as *mut c_void).unwrap(),
        )
    };
    if st == 0 {
        Some(unsafe { v.assume_init() })
    } else {
        None
    }
}

pub fn set<T: Copy>(id: ObjId, a: &AudioObjectPropertyAddress, v: T) -> bool {
    let st =
        unsafe { AudioObjectSetPropertyData(id, NonNull::from(a), 0, std::ptr::null(), size_of::<T>() as u32, NonNull::from(&v).cast()) };
    st == 0
}

pub fn get_vec<T: Copy>(id: ObjId, a: &AudioObjectPropertyAddress) -> Vec<T> {
    let mut size: u32 = 0;
    unsafe {
        if AudioObjectGetPropertyDataSize(id, NonNull::from(a), 0, std::ptr::null(), NonNull::from(&mut size)) != 0 || size == 0 {
            return Vec::new();
        }
        let n = size as usize / size_of::<T>();
        let mut v: Vec<T> = Vec::with_capacity(n);
        if AudioObjectGetPropertyData(
            id,
            NonNull::from(a),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(v.as_mut_ptr() as *mut c_void).unwrap(),
        ) != 0
        {
            return Vec::new();
        }
        v.set_len(size as usize / size_of::<T>());
        v
    }
}

pub fn get_string(id: ObjId, a: &AudioObjectPropertyAddress) -> Option<String> {
    let p: *const c_void = get(id, a)?;
    if p.is_null() {
        return None;
    }
    // CFStringRef is toll-free bridged with NSString; the HAL hands us a +1 reference.
    let s: Retained<NSString> = unsafe { Retained::from_raw(p as *mut NSString)? };
    Some(s.to_string())
}

// ------------------------------------------------------------------ devices

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: ObjId,
    pub uid: String,
    pub name: String,
    pub out_channels: u32,
    pub in_channels: u32,
    pub transport: u32,
    pub sample_rate: f64,
    pub hidden: bool,
}

impl Device {
    pub fn is_bluetooth(&self) -> bool {
        self.transport == kAudioDeviceTransportTypeBluetooth || self.transport == kAudioDeviceTransportTypeBluetoothLE
    }
    pub fn is_airplay(&self) -> bool {
        self.transport == kAudioDeviceTransportTypeAirPlay
    }
    pub fn is_aggregate(&self) -> bool {
        self.transport == kAudioDeviceTransportTypeAggregate || self.transport == kAudioDeviceTransportTypeAutoAggregate
    }
    pub fn transport_label(&self) -> &'static str {
        match self.transport {
            t if t == kAudioDeviceTransportTypeBuiltIn => "Built-in",
            t if t == kAudioDeviceTransportTypeUSB => "USB",
            t if t == kAudioDeviceTransportTypeBluetooth || t == kAudioDeviceTransportTypeBluetoothLE => "Bluetooth",
            t if t == kAudioDeviceTransportTypeAirPlay => "AirPlay",
            t if t == kAudioDeviceTransportTypeHDMI => "HDMI",
            t if t == kAudioDeviceTransportTypeDisplayPort => "DisplayPort",
            t if t == kAudioDeviceTransportTypeThunderbolt => "Thunderbolt",
            t if t == kAudioDeviceTransportTypeVirtual => "Virtual",
            t if t == kAudioDeviceTransportTypeAggregate || t == kAudioDeviceTransportTypeAutoAggregate => "Aggregate",
            t if t == kAudioDeviceTransportTypeFireWire => "FireWire",
            t if t == kAudioDeviceTransportTypePCI => "PCI",
            _ => "Other",
        }
    }
}

pub fn channel_count(id: ObjId, scope: u32) -> u32 {
    let a = addr(kAudioDevicePropertyStreamConfiguration, scope, ELEM_MAIN);
    let mut size: u32 = 0;
    unsafe {
        if AudioObjectGetPropertyDataSize(id, NonNull::from(&a), 0, std::ptr::null(), NonNull::from(&mut size)) != 0 || size == 0 {
            return 0;
        }
        let mut buf = vec![0u8; size as usize + 16];
        if AudioObjectGetPropertyData(
            id,
            NonNull::from(&a),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(buf.as_mut_ptr() as *mut c_void).unwrap(),
        ) != 0
        {
            return 0;
        }
        let list = &*(buf.as_ptr() as *const objc2_core_audio_types::AudioBufferList);
        let bufs = std::slice::from_raw_parts(list.mBuffers.as_ptr(), list.mNumberBuffers as usize);
        bufs.iter().map(|b| b.mNumberChannels).sum()
    }
}

pub fn device_info(id: ObjId) -> Option<Device> {
    let uid = get_string(id, &addr(kAudioDevicePropertyDeviceUID, SCOPE_GLOBAL, ELEM_MAIN))?;
    let name = get_string(id, &addr(kAudioObjectPropertyName, SCOPE_GLOBAL, ELEM_MAIN)).unwrap_or_else(|| uid.clone());
    Some(Device {
        id,
        uid,
        name,
        out_channels: channel_count(id, SCOPE_OUT),
        in_channels: channel_count(id, SCOPE_IN),
        transport: get::<u32>(id, &addr(kAudioDevicePropertyTransportType, SCOPE_GLOBAL, ELEM_MAIN)).unwrap_or(0),
        sample_rate: get::<f64>(id, &addr(kAudioDevicePropertyNominalSampleRate, SCOPE_GLOBAL, ELEM_MAIN)).unwrap_or(0.0),
        hidden: get::<u32>(id, &addr(kAudioDevicePropertyIsHidden, SCOPE_GLOBAL, ELEM_MAIN)).unwrap_or(0) != 0,
    })
}

pub fn devices() -> Vec<Device> {
    let ids: Vec<ObjId> = get_vec(SYSTEM, &addr(kAudioHardwarePropertyDevices, SCOPE_GLOBAL, ELEM_MAIN));
    ids.into_iter().filter_map(device_info).collect()
}

pub fn device_by_uid(uid: &str) -> Option<ObjId> {
    let s = NSString::from_str(uid);
    let p: *const NSString = &*s;
    let id: ObjId = get_with_qualifier(SYSTEM, &addr(kAudioHardwarePropertyTranslateUIDToDevice, SCOPE_GLOBAL, ELEM_MAIN), &p)?;
    (id != 0).then_some(id)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Output,
    Input,
    SoundEffects,
}

fn role_selector(r: Role) -> u32 {
    match r {
        Role::Output => kAudioHardwarePropertyDefaultOutputDevice,
        Role::Input => kAudioHardwarePropertyDefaultInputDevice,
        Role::SoundEffects => kAudioHardwarePropertyDefaultSystemOutputDevice,
    }
}

pub fn default_device(r: Role) -> ObjId {
    get::<ObjId>(SYSTEM, &addr(role_selector(r), SCOPE_GLOBAL, ELEM_MAIN)).unwrap_or(0)
}

pub fn set_default_device(r: Role, id: ObjId) -> bool {
    set(SYSTEM, &addr(role_selector(r), SCOPE_GLOBAL, ELEM_MAIN), id)
}

fn service_get_f32(id: ObjId, a: &AudioObjectPropertyAddress) -> Option<f32> {
    unsafe {
        if AudioHardwareServiceHasProperty(id, a) == 0 {
            return None;
        }
        let mut v: f32 = 0.0;
        let mut size = 4u32;
        (AudioHardwareServiceGetPropertyData(id, a, 0, std::ptr::null(), &mut size, &mut v as *mut f32 as *mut c_void) == 0).then_some(v)
    }
}

fn service_set_f32(id: ObjId, a: &AudioObjectPropertyAddress, v: f32) -> bool {
    unsafe {
        let mut ok: u8 = 0;
        if AudioHardwareServiceHasProperty(id, a) == 0 || AudioHardwareServiceIsPropertySettable(id, a, &mut ok) != 0 || ok == 0 {
            return false;
        }
        AudioHardwareServiceSetPropertyData(id, a, 0, std::ptr::null(), 4, &v as *const f32 as *const c_void) == 0
    }
}

fn scope_of(input: bool) -> u32 {
    if input {
        SCOPE_IN
    } else {
        SCOPE_OUT
    }
}

/// Device volume 0..1 (None when the device has no volume control, e.g. HDMI or some USB DACs).
pub fn volume(id: ObjId, input: bool) -> Option<f32> {
    let sc = scope_of(input);
    if let Some(v) = service_get_f32(id, &addr(VIRTUAL_MAIN_VOLUME, sc, ELEM_MAIN)) {
        return Some(v);
    }
    let a = addr(kAudioDevicePropertyVolumeScalar, sc, ELEM_MAIN);
    if has(id, &a) {
        return get::<f32>(id, &a);
    }
    let mut vals = Vec::new();
    for ch in 1..=2u32 {
        let a = addr(kAudioDevicePropertyVolumeScalar, sc, ch);
        if has(id, &a) {
            if let Some(v) = get::<f32>(id, &a) {
                vals.push(v);
            }
        }
    }
    (!vals.is_empty()).then(|| vals.iter().sum::<f32>() / vals.len() as f32)
}

pub fn set_volume(id: ObjId, input: bool, v: f32) -> bool {
    let sc = scope_of(input);
    let v = v.clamp(0.0, 1.0);
    if service_set_f32(id, &addr(VIRTUAL_MAIN_VOLUME, sc, ELEM_MAIN), v) {
        return true;
    }
    let a = addr(kAudioDevicePropertyVolumeScalar, sc, ELEM_MAIN);
    if has(id, &a) && settable(id, &a) {
        return set(id, &a, v);
    }
    let mut any = false;
    for ch in 1..=2u32 {
        let a = addr(kAudioDevicePropertyVolumeScalar, sc, ch);
        if has(id, &a) && settable(id, &a) {
            any |= set(id, &a, v);
        }
    }
    any
}

pub fn balance(id: ObjId) -> Option<f32> {
    service_get_f32(id, &addr(VIRTUAL_MAIN_BALANCE, SCOPE_OUT, ELEM_MAIN)).map(|b| b * 2.0 - 1.0)
}

pub fn set_balance(id: ObjId, b: f32) -> bool {
    service_set_f32(id, &addr(VIRTUAL_MAIN_BALANCE, SCOPE_OUT, ELEM_MAIN), (b.clamp(-1.0, 1.0) + 1.0) * 0.5)
}

pub fn muted(id: ObjId, input: bool) -> Option<bool> {
    let a = addr(kAudioDevicePropertyMute, scope_of(input), ELEM_MAIN);
    if has(id, &a) {
        get::<u32>(id, &a).map(|m| m != 0)
    } else {
        None
    }
}

pub fn set_muted(id: ObjId, input: bool, m: bool) -> bool {
    let a = addr(kAudioDevicePropertyMute, scope_of(input), ELEM_MAIN);
    has(id, &a) && set(id, &a, m as u32)
}

pub fn buffer_frame_range(id: ObjId) -> (u32, u32) {
    #[derive(Clone, Copy)]
    #[repr(C)]
    struct Range {
        min: f64,
        max: f64,
    }
    get::<Range>(id, &addr(kAudioDevicePropertyBufferFrameSizeRange, SCOPE_GLOBAL, ELEM_MAIN))
        .map(|r| (r.min as u32, r.max as u32))
        .unwrap_or((32, 4096))
}

pub fn set_buffer_frames(id: ObjId, frames: u32) -> bool {
    let (lo, hi) = buffer_frame_range(id);
    set(id, &addr(kAudioDevicePropertyBufferFrameSize, SCOPE_GLOBAL, ELEM_MAIN), frames.clamp(lo.max(16), hi.max(16)))
}

pub fn buffer_frames(id: ObjId) -> u32 {
    get::<u32>(id, &addr(kAudioDevicePropertyBufferFrameSize, SCOPE_GLOBAL, ELEM_MAIN)).unwrap_or(0)
}

pub fn sample_rate(id: ObjId) -> f64 {
    get::<f64>(id, &addr(kAudioDevicePropertyNominalSampleRate, SCOPE_GLOBAL, ELEM_MAIN)).unwrap_or(0.0)
}

/// Output latency in frames as the HAL reports it (device + safety offset + buffer).
pub fn output_latency_frames(id: ObjId) -> u32 {
    let dev = get::<u32>(id, &addr(kAudioDevicePropertyLatency, SCOPE_OUT, ELEM_MAIN)).unwrap_or(0);
    let safety = get::<u32>(id, &addr(kAudioDevicePropertySafetyOffset, SCOPE_OUT, ELEM_MAIN)).unwrap_or(0);
    let streams: Vec<ObjId> = get_vec(id, &addr(kAudioDevicePropertyStreams, SCOPE_OUT, ELEM_MAIN));
    let stream_lat = streams.first().and_then(|s| get::<u32>(*s, &addr(kAudioStreamPropertyLatency, SCOPE_GLOBAL, ELEM_MAIN))).unwrap_or(0);
    dev + safety + stream_lat + buffer_frames(id)
}

/// The 1-based channels the user chose for stereo in Audio MIDI Setup.
pub fn stereo_channels(id: ObjId) -> (u32, u32) {
    get::<[u32; 2]>(id, &addr(kAudioDevicePropertyPreferredChannelsForStereo, SCOPE_OUT, ELEM_MAIN))
        .filter(|c| c[0] >= 1 && c[1] >= 1)
        .map(|c| (c[0], c[1]))
        .unwrap_or((1, 2))
}

// ------------------------------------------------------------------ processes

#[derive(Clone, Debug)]
pub struct Process {
    pub object: ObjId,
    pub pid: i32,
    pub bundle_id: String,
    pub running_output: bool,
}

pub fn processes() -> Vec<Process> {
    let ids: Vec<ObjId> = get_vec(SYSTEM, &addr(kAudioHardwarePropertyProcessObjectList, SCOPE_GLOBAL, ELEM_MAIN));
    ids.into_iter()
        .filter_map(|o| {
            let pid = get::<i32>(o, &addr(kAudioProcessPropertyPID, SCOPE_GLOBAL, ELEM_MAIN))?;
            let bundle_id = get_string(o, &addr(kAudioProcessPropertyBundleID, SCOPE_GLOBAL, ELEM_MAIN)).unwrap_or_default();
            let running_output = get::<u32>(o, &addr(kAudioProcessPropertyIsRunningOutput, SCOPE_GLOBAL, ELEM_MAIN)).unwrap_or(0) != 0;
            Some(Process { object: o, pid, bundle_id, running_output })
        })
        .collect()
}

pub fn process_object_for_pid(pid: i32) -> ObjId {
    get_with_qualifier::<ObjId, i32>(SYSTEM, &addr(kAudioHardwarePropertyTranslatePIDToProcessObject, SCOPE_GLOBAL, ELEM_MAIN), &pid)
        .unwrap_or(0)
}

pub fn responsible_pid(pid: i32) -> i32 {
    let r = unsafe { responsibility_get_pid_responsible_for_pid(pid) };
    if r > 0 {
        r
    } else {
        pid
    }
}

pub fn pid_name(pid: i32) -> String {
    let mut buf = [0u8; 256];
    let n = unsafe { proc_name(pid, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
    if n > 0 {
        String::from_utf8_lossy(&buf[..n as usize]).into_owned()
    } else {
        format!("Process {pid}")
    }
}

// ------------------------------------------------------------------ listeners

pub type ListenerFn = unsafe extern "C-unwind" fn(ObjId, u32, NonNull<AudioObjectPropertyAddress>, *mut c_void) -> i32;

/// # Safety
/// `data` is handed to `f` on another thread; it must stay valid until `remove_listener`.
pub unsafe fn add_listener(id: ObjId, a: &AudioObjectPropertyAddress, f: ListenerFn, data: *mut c_void) -> bool {
    unsafe { AudioObjectAddPropertyListener(id, NonNull::from(a), Some(f), data) == 0 }
}

/// # Safety
/// Must match an earlier `add_listener` call with the same arguments.
pub unsafe fn remove_listener(id: ObjId, a: &AudioObjectPropertyAddress, f: ListenerFn, data: *mut c_void) {
    unsafe {
        AudioObjectRemovePropertyListener(id, NonNull::from(a), Some(f), data);
    }
}
