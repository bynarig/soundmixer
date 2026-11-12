// Output and Input sections of the main window.

import SwiftUI
import UniformTypeIdentifiers

struct OutputSection: View {
    @Environment(Mixer.self) private var mixer
    let snapshot: Snapshot
    @State private var importing = false

    var body: some View {
        Section {
            DevicePicker(title: "Device", role: "output", devices: snapshot.outputs, selection: snapshot.defaultOutput)
            if let out = snapshot.output {
                VolumeRow(
                    label: "Volume", muted: out.muted, volume: out.volume,
                    mutedSymbol: "speaker.slash.fill", symbol: "speaker.wave.2.fill",
                    setMuted: { mixer.setDeviceMuted(input: false, $0) },
                    setVolume: { mixer.setDeviceVolume(input: false, $0) }
                )
                if let balance = out.balance {
                    BalanceSlider(label: "Output balance", value: balance) { mixer.setOutputBalance($0) }
                }
                effects(out)
            }
        } header: {
            Text("Output")
        } footer: {
            if let out = snapshot.output {
                Text(out.detail).font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private func effects(_ out: OutputState) -> some View {
        let set = { (change: (inout DevicePrefs) -> Void) in
            var p = out.effects
            change(&p)
            mixer.setDevice(out.uid, p)
        }
        Toggle(isOn: Binding(get: { out.effects.boost }, set: { v in set { $0.boost = v } })) {
            Text("Boost")
            Text("Louder without distortion, for everything on this device")
        }
        LabeledContent {
            HStack {
                if out.effects.peqName != nil {
                    Button("Remove") { set { $0.peq.enabled = false; $0.peq.count = 0; $0.peqName = nil } }
                }
                Button("Choose File…") { importing = true }
                if out.effects.peqName != nil {
                    Toggle("Headphone EQ", isOn: Binding(get: { out.effects.peq.enabled }, set: { v in set { $0.peq.enabled = v } }))
                        .labelsHidden()
                        .toggleStyle(.switch)
                }
            }
        } label: {
            Text("Headphone EQ")
            Text(out.effects.peqName.map { "\($0) · \(out.effects.peq.count) filters" } ?? "Correct your headphones with an AutoEQ profile")
        }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.plainText, .json, .text]) { result in
            switch result {
            case .success(let url):
                let ok = url.startAccessingSecurityScopedResource()
                defer { if ok { url.stopAccessingSecurityScopedResource() } }
                if let text = try? String(contentsOf: url, encoding: .utf8) {
                    mixer.loadProfile(out.uid, text: text)
                } else {
                    mixer.notice = "That file could not be read as text."
                }
            case .failure(let e):
                mixer.notice = e.localizedDescription
            }
        }
        EqualizerRow(eq: out.effects.eq, title: "Equalizer") { eq in set { $0.eq = eq } }
    }
}

struct InputSection: View {
    @Environment(Mixer.self) private var mixer
    let snapshot: Snapshot

    var body: some View {
        Section("Input") {
            DevicePicker(title: "Device", role: "input", devices: snapshot.inputs, selection: snapshot.defaultInput)
            VolumeRow(
                label: "Input level", muted: snapshot.input.muted, volume: snapshot.input.volume,
                mutedSymbol: "mic.slash.fill", symbol: "mic.fill",
                setMuted: { mixer.setDeviceMuted(input: true, $0) },
                setVolume: { mixer.setDeviceVolume(input: true, $0) }
            )
        }
    }
}

/// Left/right balance with a centre detent.
struct BalanceSlider: View {
    let label: String
    let value: Float
    let set: (Float) -> Void

    var body: some View {
        LabeledContent("Balance") {
            Slider(
                value: Binding(get: { Double(value) }, set: { v in set(abs(v) < 0.04 ? 0 : Float(v)) }),
                in: -1...1
            ) {
                Text(label)
            } minimumValueLabel: {
                Text("L").foregroundStyle(.secondary)
            } maximumValueLabel: {
                Text("R").foregroundStyle(.secondary)
            }
            .labelsHidden()
            .frame(maxWidth: 260)
            .accessibilityValue(value == 0 ? "Centered" : value < 0 ? "\(Int(-value * 100))% left" : "\(Int(value * 100))% right")
        }
    }
}
