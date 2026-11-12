// Bridge to the Rust core: polls it 20 times a second and sends the person's changes back.

import CSoundMixer
import Foundation
import Observation

@MainActor
@Observable
final class Mixer {
    private(set) var snapshot: Snapshot?
    /// A message or problem to show once (e.g. after loading a headphone profile).
    var notice: String?

    @ObservationIgnored private var core: OpaquePointer?
    @ObservationIgnored private var timer: Timer?

    /// `audio: false` previews the window without touching any audio or saved settings.
    init(audio: Bool) {
        core = sm_new(audio)
        tick()
        timer = Timer.scheduledTimer(withTimeInterval: 0.05, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
    }

    /// Save, stop every tap and release the core (app quitting).
    func shutdown() {
        timer?.invalidate()
        timer = nil
        sm_free(core)
        core = nil
    }

    private func tick() {
        guard let core, let json = take(sm_tick(core)), let data = json.data(using: .utf8) else { return }
        do {
            snapshot = try JSONDecoder.core.decode(Snapshot.self, from: data)
        } catch {
            notice = "Could not read the audio state: \(error.localizedDescription)"
        }
    }

    // MARK: - Commands (see `core::Command` in src/core/mod.rs)

    func setApp(_ key: String, _ prefs: AppPrefs) {
        send(["cmd": "set_app", "key": key, "prefs": object(prefs)])
        update { s in
            for i in s.apps.indices where s.apps[i].key == key { s.apps[i].prefs = prefs }
            for i in s.others.indices where s.others[i].key == key { s.others[i].prefs = prefs }
        }
    }

    func setDevice(_ uid: String, _ prefs: DevicePrefs) {
        send(["cmd": "set_device", "uid": uid, "prefs": object(prefs)])
        update { $0.output?.effects = prefs }
    }

    func loadProfile(_ uid: String, text: String) {
        send(["cmd": "load_profile", "uid": uid, "text": text])
    }

    func setDefaultDevice(role: String, id: DeviceID) {
        send(["cmd": "set_default_device", "role": role, "id": id])
    }

    func setDeviceVolume(input: Bool, _ volume: Float) {
        send(["cmd": "set_device_volume", "input": input, "volume": volume])
        update { s in input ? (s.input.volume = volume) : (s.output?.volume = volume) }
    }

    func setDeviceMuted(input: Bool, _ muted: Bool) {
        send(["cmd": "set_device_muted", "input": input, "muted": muted])
        update { s in input ? (s.input.muted = muted) : (s.output?.muted = muted) }
    }

    func setOutputBalance(_ balance: Float) {
        send(["cmd": "set_output_balance", "balance": balance])
        update { $0.output?.balance = balance }
    }

    func setLatency(_ mode: String) {
        send(["cmd": "set_latency", "mode": mode])
        update { $0.latency = mode }
    }

    func setShowOtherApps(_ show: Bool) {
        send(["cmd": "set_show_other_apps", "show": show])
        update { $0.showOtherApps = show }
    }

    func resetAll() {
        send(["cmd": "reset_all"])
    }

    // MARK: - Plumbing

    /// Show a change at once instead of waiting for the next poll.
    private func update(_ change: (inout Snapshot) -> Void) {
        guard var s = snapshot else { return }
        change(&s)
        snapshot = s
    }

    private func send(_ command: [String: Any]) {
        guard let core,
              let data = try? JSONSerialization.data(withJSONObject: command),
              let json = String(data: data, encoding: .utf8)
        else { return }
        guard let reply = json.withCString({ take(sm_command(core, $0)) }),
              let obj = try? JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any]
        else { return }
        if let e = obj["error"] as? String {
            notice = e
        } else if let m = obj["message"] as? String {
            notice = m
        }
    }

    private func object<T: Encodable>(_ value: T) -> Any {
        guard let data = try? JSONEncoder.core.encode(value) else { return [String: Any]() }
        return (try? JSONSerialization.jsonObject(with: data)) ?? [String: Any]()
    }

    private func take(_ s: UnsafeMutablePointer<CChar>?) -> String? {
        guard let s else { return nil }
        defer { sm_string_free(s) }
        return String(cString: s)
    }
}
