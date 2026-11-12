// The data the Rust core sends (snapshot) and takes back (settings). Field names match
// src/core/mod.rs and src/prefs.rs; JSON keys are snake_case.

import Foundation

typealias DeviceID = UInt32

struct Snapshot: Decodable {
    var preview: Bool
    var status: String
    var outputs: [DeviceInfo]
    var inputs: [DeviceInfo]
    var defaultOutput: DeviceID
    var defaultInput: DeviceID
    var defaultEffects: DeviceID
    var output: OutputState?
    var input: InputState
    var apps: [AppState]
    var others: [AppState]
    var latency: String
    var latencyOptions: [LatencyOption]
    var showOtherApps: Bool
    var maxAppVolume: Float
    var messages: [String]
}

struct DeviceInfo: Decodable, Identifiable, Hashable {
    var id: DeviceID
    var uid: String
    var name: String
    var transport: String
}

struct OutputState: Decodable {
    var uid: String
    var volume: Float?
    var muted: Bool?
    var balance: Float?
    var detail: String
    var effects: DevicePrefs
}

struct InputState: Decodable {
    var volume: Float?
    var muted: Bool?
}

struct LatencyOption: Decodable, Hashable {
    var mode: String
    var label: String
}

struct AppState: Decodable, Identifiable {
    var key: String
    var name: String
    var bundleId: String?
    var playing: Bool
    var running: Bool
    var prefs: AppPrefs
    var meter: [Float]
    var tapped: Bool
    var detail: String?
    var error: String?
    var id: String { key }
}

struct Equalizer: Codable, Equatable {
    var enabled: Bool
    /// -1...1, maps to ±12 dB
    var preamp: Float
    /// 10 bands, -1...1 each
    var bands: [Float]

    static let rangeDB: Float = 12
    static let frequencies: [Double] = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000]
}

struct AppPrefs: Codable, Equatable {
    var volume: Float
    var muted: Bool
    var boost: Bool
    var balance: Float
    var eq: Equalizer
    var output: String?
    var favorite: Bool
    var name: String
}

struct PeqBand: Codable, Equatable {
    var kind: String
    var freq: Float
    var gainDb: Float
    var q: Float
}

struct PeqSettings: Codable, Equatable {
    var enabled: Bool
    var preampDb: Float
    var count: Int
    var bands: [PeqBand]
}

struct DevicePrefs: Codable, Equatable {
    var eq: Equalizer
    var peq: PeqSettings
    var peqName: String?
    var boost: Bool
}

extension JSONDecoder {
    static let core: JSONDecoder = {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        return d
    }()
}

extension JSONEncoder {
    static let core: JSONEncoder = {
        let e = JSONEncoder()
        e.keyEncodingStrategy = .convertToSnakeCase
        return e
    }()
}
