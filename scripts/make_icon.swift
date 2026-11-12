// Renders the app icon (1024x1024 PNG): three mixer faders on a gradient tile.
// Usage: swift scripts/make_icon.swift out.png
import AppKit

let size: CGFloat = 1024
let out = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "icon.png"

let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size),
    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
    colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

// macOS icon grid: 824pt rounded square centred in 1024.
let inset: CGFloat = 100
let tile = NSBezierPath(roundedRect: NSRect(x: inset, y: inset, width: size - 2 * inset, height: size - 2 * inset), xRadius: 185, yRadius: 185)
NSGradient(colors: [
    NSColor(calibratedRed: 0.0, green: 0.78, blue: 0.75, alpha: 1),
    NSColor(calibratedRed: 0.13, green: 0.36, blue: 0.95, alpha: 1),
])!.draw(in: tile, angle: -70)

// Three faders: a track and a knob each, at different levels.
let track: CGFloat = 34, knobW: CGFloat = 150, knobH: CGFloat = 76
let xs: [CGFloat] = [size / 2 - 210, size / 2, size / 2 + 210]
let levels: [CGFloat] = [0.38, 0.72, 0.52]
let bottom: CGFloat = 250, top: CGFloat = size - 250
for (x, level) in zip(xs, levels) {
    NSColor(white: 1, alpha: 0.3).setFill()
    NSBezierPath(roundedRect: NSRect(x: x - track / 2, y: bottom, width: track, height: top - bottom), xRadius: track / 2, yRadius: track / 2).fill()
    let y = bottom + (top - bottom) * level
    NSColor(white: 1, alpha: 0.95).setFill()
    NSBezierPath(roundedRect: NSRect(x: x - track / 2, y: bottom, width: track, height: y - bottom), xRadius: track / 2, yRadius: track / 2).fill()
    let knob = NSBezierPath(roundedRect: NSRect(x: x - knobW / 2, y: y - knobH / 2, width: knobW, height: knobH), xRadius: knobH / 2, yRadius: knobH / 2)
    NSGraphicsContext.saveGraphicsState()
    let shadow = NSShadow()
    shadow.shadowColor = NSColor(white: 0, alpha: 0.25)
    shadow.shadowBlurRadius = 18
    shadow.shadowOffset = NSSize(width: 0, height: -6)
    shadow.set()
    NSColor.white.setFill()
    knob.fill()
    NSGraphicsContext.restoreGraphicsState()
}
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
