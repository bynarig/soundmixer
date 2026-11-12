// Pure mappings between stored values and what the controls show.

import Foundation

enum Scale {
    /// Where 100 % sits on an app's volume slider: the first three quarters go up to 100 %,
    /// the last quarter is the boost zone up to the maximum.
    static let unity: Double = 0.75

    /// Slider position (0...1) for a volume (0...max). Non-finite volume reads as 100 %.
    static func position(volume v: Float, max: Float) -> Double {
        let max = Double(max > 1 ? max : 1)
        let v = v.isFinite ? Swift.min(Swift.max(Double(v), 0), max) : 1
        if v <= 1 { return v * unity }
        return max == 1 ? 1 : unity + (v - 1) / (max - 1) * (1 - unity)
    }

    /// Volume for a slider position, snapping to exactly 100 % near the detent.
    static func volume(position p: Double, max: Float) -> Float {
        let max = Double(max > 1 ? max : 1)
        let p = p.isFinite ? Swift.min(Swift.max(p, 0), 1) : unity
        let v = p <= unity ? p / unity : 1 + (p - unity) / (1 - unity) * (max - 1)
        return abs(v - 1) < 0.02 ? 1 : Float(Swift.min(Swift.max(v, 0), max))
    }

    /// Meter fill (0...1) for a peak level, on a -60...0 dBFS scale.
    static func meter(_ peak: Float) -> Double {
        guard peak.isFinite, peak > 0 else { return 0 }
        let db = 20 * log10(Double(peak))
        return Swift.min(Swift.max((db + 60) / 60, 0), 1)
    }

    static func percent(_ v: Float) -> String {
        "\(Int((Double(v.isFinite ? v : 0) * 100).rounded()))%"
    }

    /// 1000 → "1k", 125 → "125".
    static func frequency(_ f: Double) -> String {
        f >= 1000 ? "\(Int(f / 1000))k" : "\(Int(f))"
    }

    static func decibels(_ slider: Float) -> String {
        String(format: "%+.1f dB", slider * Equalizer.rangeDB)
    }
}
