// Applications section: one row per app with its icon, level meter and volume slider; the
// rest of its settings live in a popover.

import AppKit
import SwiftUI

struct AppsSection: View {
    @Environment(Mixer.self) private var mixer
    let snapshot: Snapshot

    var body: some View {
        Section {
            if snapshot.apps.isEmpty {
                Text("Apps appear here when they play sound.")
                    .foregroundStyle(.secondary)
            }
            ForEach(snapshot.apps) { AppRow(app: $0, snapshot: snapshot) }
        } header: {
            Text("Applications")
        }
        if !snapshot.others.isEmpty {
            Section {
                DisclosureGroup(
                    "Other Apps Using Audio (\(snapshot.others.count))",
                    isExpanded: Binding(get: { snapshot.showOtherApps }, set: { mixer.setShowOtherApps($0) })
                ) {
                    ForEach(snapshot.others) { AppRow(app: $0, snapshot: snapshot) }
                }
            }
        }
    }
}

struct AppRow: View {
    @Environment(Mixer.self) private var mixer
    let app: AppState
    let snapshot: Snapshot
    @State private var showingOptions = false

    private var prefs: AppPrefs { app.prefs }

    private func set(_ change: (inout AppPrefs) -> Void) {
        var p = prefs
        change(&p)
        mixer.setApp(app.key, p)
    }

    var body: some View {
        HStack(spacing: 12) {
            AppIcon(bundleID: app.bundleId)
                .frame(width: 32, height: 32)
                .opacity(app.running ? 1 : 0.5)
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text(app.name.isEmpty ? app.key : app.name)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundStyle(app.running ? .primary : .secondary)
                    if prefs.favorite {
                        Image(systemName: "star.fill").font(.caption).foregroundStyle(.yellow).accessibilityLabel("Favorite")
                    }
                    if prefs.boost {
                        Text("Boost").font(.caption).foregroundStyle(.orange)
                    }
                    Spacer(minLength: 4)
                    Text(Scale.percent(prefs.volume))
                        .monospacedDigit()
                        .foregroundStyle(prefs.volume > 1 ? AnyShapeStyle(.orange) : AnyShapeStyle(.secondary))
                        .onTapGesture(count: 2) { set { $0.volume = 1 } }
                        .help("Double-click to reset to 100%")
                }
                HStack(spacing: 8) {
                    Button(prefs.muted ? "Unmute \(app.name)" : "Mute \(app.name)",
                           systemImage: prefs.muted ? "speaker.slash.fill" : "speaker.wave.2.fill") {
                        set { $0.muted.toggle() }
                    }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .contentTransition(.symbolEffect(.replace))
                    .foregroundStyle(prefs.muted ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                    .frame(width: 20)
                    Slider(value: Binding(
                        get: { Scale.position(volume: prefs.volume, max: snapshot.maxAppVolume) },
                        set: { p in set { $0.volume = Scale.volume(position: p, max: snapshot.maxAppVolume) } }
                    ), in: 0...1)
                    .labelsHidden()
                    .tint(prefs.volume > 1 ? .orange : .accentColor)
                    .accessibilityLabel("\(app.name) volume")
                    .accessibilityValue(Scale.percent(prefs.volume))
                }
                LevelMeter(level: Scale.meter(max(app.meter.first ?? 0, app.meter.last ?? 0)), active: app.tapped)
                if let error = app.error {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            }
            Button("Options for \(app.name)", systemImage: "slider.horizontal.3") { showingOptions = true }
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
                .help("Output, balance, equalizer and more")
                .popover(isPresented: $showingOptions, arrowEdge: .trailing) {
                    AppOptions(app: app, outputs: snapshot.outputs, set: set)
                }
        }
        .padding(.vertical, 2)
        .contextMenu {
            Button(prefs.favorite ? "Remove from Favorites" : "Add to Favorites") { set { $0.favorite.toggle() } }
            Button(prefs.muted ? "Unmute" : "Mute") { set { $0.muted.toggle() } }
            Button("Reset to 100%") { set { $0.volume = 1 } }
        }
    }
}

/// Everything else about one app, in a popover.
struct AppOptions: View {
    let app: AppState
    let outputs: [DeviceInfo]
    let set: ((inout AppPrefs) -> Void) -> Void

    var body: some View {
        let p = app.prefs
        Form {
            Section {
                Picker("Output", selection: Binding(get: { p.output }, set: { v in set { $0.output = v } })) {
                    Text("Match System Output").tag(String?.none)
                    Divider()
                    if let uid = p.output, !outputs.contains(where: { $0.uid == uid }) {
                        Text("Unplugged Device").tag(String?.some(uid))
                    }
                    ForEach(outputs) { d in
                        Label(d.name, systemImage: DeviceSymbol.name(transport: d.transport, input: false)).tag(String?.some(d.uid))
                    }
                }
                BalanceSlider(label: "\(app.name) balance", value: p.balance) { v in set { $0.balance = v } }
                Toggle(isOn: Binding(get: { p.boost }, set: { v in set { $0.boost = v } })) {
                    Text("Boost")
                    Text("Louder without distortion")
                }
                EqualizerRow(eq: p.eq, title: "Equalizer") { eq in set { $0.eq = eq } }
                Toggle(isOn: Binding(get: { p.favorite }, set: { v in set { $0.favorite = v } })) {
                    Text("Always Show")
                    Text("Keep this app in the list when it is quiet")
                }
            } header: {
                Text(app.name)
            } footer: {
                if let d = app.detail {
                    Text(d).font(.caption).foregroundStyle(.secondary)
                } else {
                    Text("Plays directly, with no added latency, until you change something.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 380)
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// A thin level meter under the slider (-60...0 dBFS).
struct LevelMeter: View {
    let level: Double
    let active: Bool

    var body: some View {
        GeometryReader { g in
            Capsule()
                .fill(.quaternary)
                .overlay(alignment: .leading) {
                    Capsule()
                        .fill(level > 0.95 ? AnyShapeStyle(.red) : level > 0.8 ? AnyShapeStyle(.orange) : AnyShapeStyle(.green))
                        .frame(width: g.size.width * level)
                }
        }
        .frame(height: 3)
        .opacity(active ? 1 : 0.4)
        .accessibilityHidden(true)
    }
}

/// The app's own icon from its bundle, or a generic app icon.
struct AppIcon: View {
    let bundleID: String?

    @MainActor private static var cache: [String: NSImage] = [:]

    var body: some View {
        Image(nsImage: Self.icon(for: bundleID))
            .resizable()
            .interpolation(.high)
            .accessibilityHidden(true)
    }

    @MainActor private static func icon(for id: String?) -> NSImage {
        let key = id ?? ""
        if let cached = cache[key] { return cached }
        let image: NSImage
        if let id, let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: id) {
            image = NSWorkspace.shared.icon(forFile: url.path)
        } else {
            image = NSWorkspace.shared.icon(for: .application)
        }
        cache[key] = image
        return image
    }
}
