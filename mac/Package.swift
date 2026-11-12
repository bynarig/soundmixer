// swift-tools-version: 6.0
import Foundation
import PackageDescription

// The Rust core is built first (`cargo build --release` in the repo root, see build-app.sh).
let rustLib = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("../target/release").standardized.path

let package = Package(
    name: "SoundMixer",
    platforms: [.macOS(.v14)],
    targets: [
        .systemLibrary(name: "CSoundMixer", path: "Sources/CSoundMixer"),
        .executableTarget(
            name: "SoundMixer",
            dependencies: ["CSoundMixer"],
            linkerSettings: [
                .unsafeFlags(["-L", rustLib]),
                .linkedFramework("AppKit"),
                .linkedFramework("CoreAudio"),
                .linkedFramework("AudioToolbox"),
                .linkedLibrary("iconv"),
            ]
        ),
        .testTarget(name: "SoundMixerTests", dependencies: ["SoundMixer"]),
    ]
)
