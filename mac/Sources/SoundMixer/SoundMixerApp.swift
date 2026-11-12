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

        Settings {
            SettingsView()
                .environment(delegate.mixer)
        }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// `--preview` shows the window without touching any audio or saved settings.
    let mixer = Mixer(audio: !CommandLine.arguments.contains("--preview"))

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }

    func applicationWillTerminate(_ notification: Notification) {
        mixer.shutdown()
    }
}
