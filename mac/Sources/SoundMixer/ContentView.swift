// The main window: a grouped form like System Settings, with Output, Input, Sound Effects and
// Applications sections.

import SwiftUI

struct ContentView: View {
    @Environment(Mixer.self) private var mixer
    @State private var confirmReset = false

    var body: some View {
        Group {
            if let s = mixer.snapshot {
                Form {
                    if !s.messages.isEmpty {
                        Section {
                            ForEach(s.messages, id: \.self) { Label($0, systemImage: "exclamationmark.triangle").foregroundStyle(.secondary) }
                        }
                    }
                    OutputSection(snapshot: s)
                    InputSection(snapshot: s)
                    Section("Sound Effects") {
                        DevicePicker(title: "Play alerts through", role: "sound_effects", devices: s.outputs, selection: s.defaultEffects)
                    }
                    AppsSection(snapshot: s)
                }
                .formStyle(.grouped)
            } else {
                ProgressView()
            }
        }
        .navigationTitle("SoundMixer")
        .navigationSubtitle(mixer.snapshot?.status ?? "")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Reset All", systemImage: "arrow.counterclockwise") { confirmReset = true }
                    .help("Put every app back to 100% with no effects")
            }
        }
        .confirmationDialog("Reset all apps?", isPresented: $confirmReset) {
            Button("Reset All", role: .destructive) { mixer.resetAll() }
        } message: {
            Text("Every app goes back to 100% volume with no effects. Favorites stay in the list.")
        }
        .alert(
            mixer.notice ?? "",
            isPresented: Binding(get: { mixer.notice != nil }, set: { if !$0 { mixer.notice = nil } })
        ) {
            Button("OK") {}
        }
    }
}

struct SettingsView: View {
    @Environment(Mixer.self) private var mixer

    var body: some View {
        Form {
            if let s = mixer.snapshot {
                Picker("Latency", selection: Binding(get: { s.latency }, set: { mixer.setLatency($0) })) {
                    ForEach(s.latencyOptions, id: \.mode) { Text($0.label).tag($0.mode) }
                }
                Text("Automatic picks a buffer for each device. Choose Glitch-proof if you hear crackles.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .formStyle(.grouped)
        .frame(width: 420)
        .fixedSize(horizontal: false, vertical: true)
    }
}

// MARK: - Shared pieces

/// A device menu that changes the system default device.
struct DevicePicker: View {
    @Environment(Mixer.self) private var mixer
    let title: String
    let role: String
    let devices: [DeviceInfo]
    let selection: DeviceID

    var body: some View {
        Picker(title, selection: Binding(get: { selection }, set: { mixer.setDefaultDevice(role: role, id: $0) })) {
            if !devices.contains(where: { $0.id == selection }) {
                Text("None").tag(selection)
            }
            ForEach(devices) { d in
                Label(d.name, systemImage: DeviceSymbol.name(transport: d.transport, input: role == "input")).tag(d.id)
            }
        }
    }
}

enum DeviceSymbol {
    static func name(transport: String, input: Bool) -> String {
        switch transport {
        case "Built-in": input ? "mic" : "laptopcomputer"
        case "Bluetooth": "headphones"
        case "AirPlay": "airplayaudio"
        case "USB", "Thunderbolt", "FireWire": "cable.connector"
        case "HDMI", "DisplayPort": "tv"
        case "Virtual", "Aggregate": "square.stack.3d.up"
        default: input ? "mic" : "hifispeaker"
        }
    }
}

/// Mute button and volume slider, as in Control Center.
struct VolumeRow: View {
    let label: String
    let muted: Bool?
    let volume: Float?
    let mutedSymbol: String
    let symbol: String
    let setMuted: (Bool) -> Void
    let setVolume: (Float) -> Void

    var body: some View {
        if let volume {
            LabeledContent(label) {
                HStack(spacing: 8) {
                let isMuted = muted ?? false
                Button(isMuted ? "Unmute" : "Mute", systemImage: isMuted ? mutedSymbol : symbol) { setMuted(!isMuted) }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .contentTransition(.symbolEffect(.replace))
                    .frame(width: 22)
                    .disabled(muted == nil)
                    .help(isMuted ? "Unmute" : "Mute")
                Slider(value: Binding(get: { Double(volume) }, set: { setVolume(Float($0)) }), in: 0...1)
                    .labelsHidden()
                    .accessibilityLabel(label)
                    .accessibilityValue(Scale.percent(volume))
                Text(Scale.percent(volume))
                    .monospacedDigit()
                    .foregroundStyle(.secondary)
                    .frame(width: 40, alignment: .trailing)
                }
                .frame(maxWidth: 260)
            }
        } else {
            LabeledContent(label) { Text("Not adjustable").foregroundStyle(.secondary) }
        }
    }
}
