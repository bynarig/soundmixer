import Foundation
import Testing
@testable import SoundMixer

struct ScaleTests {
    let max: Float = 4

    @Test func anchorPoints() {
        #expect(Scale.position(volume: 0, max: max) == 0)
        #expect(Scale.position(volume: 1, max: max) == Scale.unity)
        #expect(Scale.position(volume: max, max: max) == 1)
        #expect(Scale.volume(position: 0, max: max) == 0)
        #expect(Scale.volume(position: Scale.unity, max: max) == 1)
        #expect(Scale.volume(position: 1, max: max) == max)
    }

    @Test func badInputIsSafe() {
        for v in [Float.nan, .infinity, -.infinity] {
            #expect(Scale.position(volume: v, max: max) == Scale.unity)
        }
        for p in [Double.nan, .infinity, -.infinity] {
            #expect(Scale.volume(position: p, max: max) == 1)
        }
        #expect(Scale.position(volume: -3, max: max) == 0)
        #expect(Scale.position(volume: 1e9, max: max) == 1)
        #expect(Scale.volume(position: 7, max: max) == max)
        // a broken maximum never divides by zero
        #expect(Scale.position(volume: 2, max: 0) == Scale.unity)
        #expect(Scale.volume(position: 1, max: .nan) == 1)
    }

    @Test func unityDetent() {
        #expect(Scale.volume(position: Scale.position(volume: 0.981, max: max), max: max) == 1)
        #expect(Scale.volume(position: Scale.position(volume: 1.019, max: max), max: max) == 1)
        #expect(abs(Scale.volume(position: Scale.position(volume: 0.97, max: max), max: max) - 0.97) < 1e-4)
        #expect(abs(Scale.volume(position: Scale.position(volume: 1.03, max: max), max: max) - 1.03) < 1e-4)
    }

    @Test func monotonicAndRoundTrips() {
        var prev: Float = -1
        for i in 0...4000 {
            let v = Scale.volume(position: Double(i) / 4000, max: max)
            #expect(v >= prev)
            prev = v
            let vv = Float(i) / 1000
            if abs(vv - 1) >= 0.02 {
                #expect(abs(Scale.volume(position: Scale.position(volume: vv, max: max), max: max) - vv) < 1e-4)
            }
        }
    }

    @Test func meterAndLabels() {
        #expect(Scale.meter(0) == 0)
        #expect(Scale.meter(.nan) == 0)
        #expect(Scale.meter(-1) == 0)
        #expect(Scale.meter(1) == 1)
        #expect(Scale.meter(100) == 1)
        #expect(Scale.meter(0.001) < 1e-6, "-60 dB is empty")
        #expect(Scale.percent(1) == "100%")
        #expect(Scale.percent(.nan) == "0%")
        #expect(Equalizer.frequencies.map(Scale.frequency) == ["32", "64", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"])
        #expect(Scale.decibels(-1) == "-12.0 dB")
    }

    @Test func presetsAreInRange() {
        for p in EqualizerEditor.presets {
            #expect(p.bands.count == 10)
            #expect(p.bands.allSatisfy { (-1...1).contains($0) }, "\(p.name)")
        }
    }
}

struct ModelTests {
    /// A real snapshot from the Rust core (preview mode) decodes, and settings encode back in
    /// the shape Rust expects.
    @Test @MainActor func coreSnapshotDecodesAndSettingsRoundTrip() throws {
        let mixer = Mixer(audio: false)
        defer { mixer.shutdown() }
        let s = try #require(mixer.snapshot, "\(mixer.notice ?? "")")
        #expect(s.preview)
        #expect(s.latencyOptions.count == 4)
        #expect(s.maxAppVolume == 4)
        if let out = s.output {
            let data = try JSONEncoder.core.encode(out.effects)
            let json = try #require(String(data: data, encoding: .utf8))
            #expect(json.contains("\"preamp_db\""))
            #expect(try JSONDecoder.core.decode(DevicePrefs.self, from: data) == out.effects)
        }
    }

    @Test @MainActor func badCommandsBecomeNotices() {
        let mixer = Mixer(audio: false)
        defer { mixer.shutdown() }
        mixer.loadProfile("nope", text: "garbage")
        #expect(mixer.notice != nil)
    }
}
