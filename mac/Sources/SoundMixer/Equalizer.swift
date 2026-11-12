// 10-band equalizer: an on/off row that opens the band sliders in a popover.

import SwiftUI

struct EqualizerRow: View {
    let eq: Equalizer
    let title: String
    let set: (Equalizer) -> Void
    @State private var editing = false

    var body: some View {
        LabeledContent {
            HStack {
                Button("Edit…") { editing = true }
                    .popover(isPresented: $editing, arrowEdge: .bottom) {
                        EqualizerEditor(eq: eq, set: set)
                    }
                Toggle(title, isOn: Binding(get: { eq.enabled }, set: { v in var e = eq; e.enabled = v; set(e) }))
                    .labelsHidden()
                    .toggleStyle(.switch)
            }
        } label: {
            Text(title)
            Text(summary).foregroundStyle(.secondary)
        }
    }

    private var summary: String {
        if !eq.enabled { return "Off" }
        if let p = EqualizerEditor.presets.first(where: { $0.bands == eq.bands && eq.preamp == 0 }) { return p.name }
        return "Custom"
    }
}

struct EqualizerEditor: View {
    let eq: Equalizer
    let set: (Equalizer) -> Void

    nonisolated static let presets: [(name: String, bands: [Float])] = [
        ("Flat", Array(repeating: 0, count: 10)),
        ("Bass Boost", [0.5, 0.42, 0.3, 0.12, 0, 0, 0, 0, 0, 0]),
        ("Bass Reducer", [-0.5, -0.42, -0.3, -0.12, 0, 0, 0, 0, 0, 0]),
        ("Treble Boost", [0, 0, 0, 0, 0, 0.05, 0.15, 0.3, 0.42, 0.5]),
        ("Vocal", [-0.4, -0.3, -0.1, 0.05, 0.2, 0.3, 0.3, 0.2, 0, -0.1]),
        ("Loudness", [0.4, 0.3, 0.1, 0, -0.05, 0, 0, 0.1, 0.3, 0.35]),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Toggle("Equalizer", isOn: binding(\.enabled))
                    .toggleStyle(.switch)
                Spacer()
                Menu("Presets") {
                    ForEach(Self.presets, id: \.name) { p in
                        Button(p.name) {
                            var e = eq
                            e.bands = p.bands
                            e.preamp = 0
                            e.enabled = true
                            set(e)
                        }
                    }
                }
                .fixedSize()
            }
            HStack(alignment: .top, spacing: 4) {
                band("Preamp", short: "Pre", value: binding(\.preamp))
                Divider().frame(height: 150)
                ForEach(Equalizer.frequencies.indices, id: \.self) { i in
                    let f = Scale.frequency(Equalizer.frequencies[i])
                    band("\(f) Hz", short: f, value: Binding(
                        get: { eq.bands.indices.contains(i) ? eq.bands[i] : 0 },
                        set: { v in var e = eq; if e.bands.indices.contains(i) { e.bands[i] = v }; set(e) }
                    ))
                }
            }
            .disabled(!eq.enabled)
        }
        .padding(16)
        .frame(width: 470)
    }

    private func binding(_ path: WritableKeyPath<Equalizer, Bool>) -> Binding<Bool> {
        Binding(get: { eq[keyPath: path] }, set: { v in var e = eq; e[keyPath: path] = v; set(e) })
    }

    private func binding(_ path: WritableKeyPath<Equalizer, Float>) -> Binding<Float> {
        Binding(get: { eq[keyPath: path] }, set: { v in var e = eq; e[keyPath: path] = v; set(e) })
    }

    /// One vertical slider (a standard Slider turned upright) with its label.
    private func band(_ name: String, short: String, value: Binding<Float>) -> some View {
        VStack(spacing: 6) {
            Slider(value: Binding(get: { Double(value.wrappedValue) }, set: { v in value.wrappedValue = abs(v) < 0.03 ? 0 : Float(v) }), in: -1...1)
                .controlSize(.small)
                .frame(width: 130)
                .rotationEffect(.degrees(-90))
                .frame(width: 30, height: 130)
                .accessibilityLabel(name)
                .accessibilityValue(Scale.decibels(value.wrappedValue))
                .help(Scale.decibels(value.wrappedValue))
            Text(short)
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
    }
}
