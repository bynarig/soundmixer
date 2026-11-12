// C interface of the Rust core (src/ffi.rs). All strings are UTF-8 JSON; free them with
// sm_string_free. Call everything from the main thread.
#pragma once
#include <stdbool.h>

typedef struct SMCore SMCore;

/// audio == false previews the UI: no taps, no device changes, no saving.
SMCore *sm_new(bool audio);
/// Poll devices and apps, update taps, return the snapshot JSON.
char *sm_tick(SMCore *core);
/// Apply one command; returns {"message": ...} or {"error": ...}.
char *sm_command(SMCore *core, const char *json);
/// Save settings, stop every tap, free the core.
void sm_free(SMCore *core);
void sm_string_free(char *s);
