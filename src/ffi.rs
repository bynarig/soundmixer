//! C interface for the SwiftUI app (`mac/Sources/CSoundMixer/soundmixer.h`). Strings cross as
//! UTF-8 JSON. Every call runs on the main thread; a panic is caught here, never unwound into Swift.

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::core::{Command, Core};

fn to_c(s: String) -> *mut c_char {
    CString::new(s).map(CString::into_raw).unwrap_or(std::ptr::null_mut())
}

fn error_json(e: impl std::fmt::Display) -> String {
    serde_json::json!({ "error": e.to_string() }).to_string()
}

/// Start the core. `audio == false` previews the UI without touching any audio. Free with `sm_free`.
#[no_mangle]
pub extern "C" fn sm_new(audio: bool) -> *mut Core {
    catch_unwind(|| Box::into_raw(Box::new(Core::new(audio)))).unwrap_or(std::ptr::null_mut())
}

/// Poll devices and apps, update taps, and return the snapshot JSON (free with `sm_string_free`).
///
/// # Safety
/// `core` must come from `sm_new` and not be freed.
#[no_mangle]
pub unsafe extern "C" fn sm_tick(core: *mut Core) -> *mut c_char {
    let Some(core) = (unsafe { core.as_mut() }) else { return std::ptr::null_mut() };
    let out = catch_unwind(AssertUnwindSafe(|| {
        core.tick();
        serde_json::to_string(&core.snapshot()).unwrap_or_else(error_json)
    }));
    to_c(out.unwrap_or_else(|_| error_json("internal error")))
}

/// Apply one command (JSON, see `core::Command`). Returns `{"message": …}` or `{"error": …}`
/// (free with `sm_string_free`).
///
/// # Safety
/// `core` must come from `sm_new`; `json` must be a NUL-terminated string or null.
#[no_mangle]
pub unsafe extern "C" fn sm_command(core: *mut Core, json: *const c_char) -> *mut c_char {
    let Some(core) = (unsafe { core.as_mut() }) else { return std::ptr::null_mut() };
    if json.is_null() {
        return to_c(error_json("no command"));
    }
    let text = unsafe { CStr::from_ptr(json) }.to_string_lossy();
    let out = catch_unwind(AssertUnwindSafe(|| match serde_json::from_str::<Command>(&text) {
        Ok(cmd) => match core.apply(cmd) {
            Ok(msg) => serde_json::json!({ "message": msg }).to_string(),
            Err(e) => error_json(e),
        },
        Err(e) => error_json(e),
    }));
    to_c(out.unwrap_or_else(|_| error_json("internal error")))
}

/// Save settings, stop every tap and free the core.
///
/// # Safety
/// `core` must come from `sm_new` (or be null) and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn sm_free(core: *mut Core) {
    if !core.is_null() {
        let mut core = unsafe { Box::from_raw(core) };
        let _ = catch_unwind(AssertUnwindSafe(|| core.shutdown()));
    }
}

/// Free a string returned by `sm_tick` or `sm_command`.
///
/// # Safety
/// `s` must come from this library (or be null) and be freed once.
#[no_mangle]
pub unsafe extern "C" fn sm_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take(s: *mut c_char) -> serde_json::Value {
        assert!(!s.is_null());
        let v = serde_json::from_str(&unsafe { CStr::from_ptr(s) }.to_string_lossy()).unwrap();
        unsafe { sm_string_free(s) };
        v
    }

    #[test]
    fn round_trip_without_audio() {
        let core = sm_new(false);
        assert!(!core.is_null());
        let snap = take(unsafe { sm_tick(core) });
        assert_eq!(snap["preview"], true);
        assert!(snap["latency_options"].as_array().unwrap().len() == 4);

        let cmd = CString::new(r#"{"cmd":"set_show_other_apps","show":true}"#).unwrap();
        assert!(take(unsafe { sm_command(core, cmd.as_ptr()) })["error"].is_null());
        // bad input is an error, not a crash
        for bad in ["", "{", "null", r#"{"cmd":"nope"}"#, r#"{"cmd":"set_device_volume","input":false}"#] {
            let c = CString::new(bad).unwrap();
            assert!(take(unsafe { sm_command(core, c.as_ptr()) })["error"].is_string(), "{bad}");
        }
        assert!(take(unsafe { sm_command(core, std::ptr::null()) })["error"].is_string());
        unsafe { sm_free(core) }; // a preview core never saves
    }

    #[test]
    fn null_core_is_ignored() {
        assert!(unsafe { sm_tick(std::ptr::null_mut()) }.is_null());
        let c = CString::new("{}").unwrap();
        assert!(unsafe { sm_command(std::ptr::null_mut(), c.as_ptr()) }.is_null());
        unsafe { sm_free(std::ptr::null_mut()) };
        unsafe { sm_string_free(std::ptr::null_mut()) };
    }
}
