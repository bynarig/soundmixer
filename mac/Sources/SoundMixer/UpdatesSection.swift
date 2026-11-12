// Settings → Updates: version, automatic checks and the update button.

import SwiftUI

/// Version, automatic checks and the update button.
struct UpdatesSection: View {
    @State private var updater = Updater.shared

    var body: some View {
        LabeledContent("Version") {
            Text(verbatim: updater.currentVersion).monospacedDigit()
        }
        VStack(alignment: .leading, spacing: 4) {
            Toggle("Check for updates automatically", isOn: $updater.automaticChecks)
            Text("SoundMixer checks when it starts and downloads new versions in the background. You choose when to restart.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .disabled(!updater.isConfigured)
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            status
            Spacer(minLength: 8)
            action
        }
    }

    @ViewBuilder private var status: some View {
        switch updater.phase {
        case .idle:
            if !updater.isConfigured {
                Text("Updates aren't set up for this build.").foregroundStyle(.secondary)
            }
        case .checking:
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Checking…").foregroundStyle(.secondary)
            }
        case .upToDate:
            Label("SoundMixer is up to date.", systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
        case .available(let info):
            VStack(alignment: .leading, spacing: 4) {
                Text("Version \(info.version) is available.").font(.headline)
                if let notes = info.notes, !notes.isEmpty {
                    Text(verbatim: notes).font(.caption).foregroundStyle(.secondary)
                }
            }
        case .downloading(let progress):
            VStack(alignment: .leading, spacing: 4) {
                Text("Downloading…").foregroundStyle(.secondary)
                ProgressView(value: progress).frame(maxWidth: 220)
            }
        case .ready(let info, _):
            VStack(alignment: .leading, spacing: 4) {
                Text("Version \(info.version) is ready to install.").font(.headline)
                if let notes = info.notes, !notes.isEmpty {
                    Text(verbatim: notes).font(.caption).foregroundStyle(.secondary)
                }
            }
        case .installing:
            HStack(spacing: 6) {
                ProgressView().controlSize(.small)
                Text("Installing…").foregroundStyle(.secondary)
            }
        case .failed(let message):
            Label(message, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder private var action: some View {
        if updater.readyUpdate != nil {
            Button("Restart to Update") { updater.installReady() }
                .buttonStyle(.borderedProminent)
        } else if let info = updater.availableUpdate {
            Button("Install and Relaunch") { Task { await updater.install(info) } }
                .buttonStyle(.borderedProminent)
        } else {
            Button("Check for Updates") { Task { await updater.check(userInitiated: true) } }
                .disabled({
                    switch updater.phase {
                    case .checking, .downloading, .ready, .installing: true
                    default: false
                    }
                }())
        }
    }
}
