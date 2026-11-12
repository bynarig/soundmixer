// App entry: one window, a Settings window (⌘,), and a clean shutdown of the audio core.
// Standard SwiftUI controls pick up Liquid Glass on macOS 26 by themselves (HIG: prefer
// system components), so there is no custom glass in the content.

import AppKit
import SwiftUI

@main
struct SoundMixerApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        Window("SoundMixer", id: "main") {
            ContentView()
                .environment(delegate.mixer)
                .frame(minWidth: 420, minHeight: 480)
        }
        .defaultSize(width: 480, height: 760)
        .windowToolbarStyle(.unified)
        .commands {
            CommandGroup(after: .appInfo) {
                Button("Check for Updates…") {
                    Task { await Updater.shared.check(userInitiated: true) }
                    NSApp.sendAction(Selector(("showSettingsWindow:")), to: nil, from: nil)
                }
            }
        }

        Settings {
            SettingsView()
                .environment(delegate.mixer)
        }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// `--preview` shows the window without touching any audio or saved settings.
    let preview = CommandLine.arguments.contains("--preview")
    lazy var mixer = Mixer(audio: !preview)

    func applicationDidFinishLaunching(_ notification: Notification) {
        // a preview never goes online
        if !preview { Updater.shared.checkOnLaunch() }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    func applicationWillTerminate(_ notification: Notification) {
        mixer.shutdown()
    }
}
