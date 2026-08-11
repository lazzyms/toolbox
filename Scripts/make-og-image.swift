#!/usr/bin/env swift

// Generates docs/og.png — the 1200×630 social preview card for the website.
// Committing a generator instead of a binary keeps the repo diffable and lets the
// card be re-rendered when the wording changes. Palette and icon geometry match
// Scripts/make-icon.swift.
//
//   swift Scripts/make-og-image.swift [output-directory]

import AppKit
import CoreGraphics
import Foundation

let outputDirectory = CommandLine.arguments.count > 1
    ? URL(fileURLWithPath: CommandLine.arguments[1])
    : URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
        .appendingPathComponent("docs")

let width = 1200, height = 630
let scale = 2  // render at 2x so the card stays sharp when scrapers downsample

let brandLight = CGColor(red: 0.29, green: 0.56, blue: 0.98, alpha: 1)
let brandDark = CGColor(red: 0.16, green: 0.31, blue: 0.83, alpha: 1)
let sRGB = CGColorSpace(name: CGColorSpace.sRGB)!

guard let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: width * scale, pixelsHigh: height * scale,
    bitsPerSample: 8, samplesPerPixel: 4,
    hasAlpha: true, isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0, bitsPerPixel: 0
) else {
    FileHandle.standardError.write(Data("could not allocate bitmap\n".utf8))
    exit(1)
}
rep.size = NSSize(width: width, height: height)

guard let nsContext = NSGraphicsContext(bitmapImageRep: rep) else {
    FileHandle.standardError.write(Data("could not make a drawing context\n".utf8))
    exit(1)
}
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = nsContext
let ctx = nsContext.cgContext

// ── background ────────────────────────────────────────────────────────────────
ctx.setFillColor(CGColor(red: 0.027, green: 0.035, blue: 0.059, alpha: 1))  // #07090F
ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))

// Blue bloom behind the headline, echoing the hero glow on the page.
func bloom(center: CGPoint, radius: CGFloat, color: CGColor, alpha: CGFloat) {
    let faded = color.copy(alpha: 0)!
    let gradient = CGGradient(
        colorsSpace: sRGB,
        colors: [color.copy(alpha: alpha)!, faded] as CFArray,
        locations: [0, 1]
    )!
    ctx.drawRadialGradient(
        gradient,
        startCenter: center, startRadius: 0,
        endCenter: center, endRadius: radius,
        options: []
    )
}
bloom(center: CGPoint(x: 250, y: 660), radius: 620, color: brandLight, alpha: 0.42)
bloom(center: CGPoint(x: 1080, y: 120), radius: 460,
      color: CGColor(red: 0.75, green: 0.35, blue: 0.95, alpha: 1), alpha: 0.20)

// Hairline frame so the card reads as a card on light backgrounds.
ctx.setStrokeColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.08))
ctx.setLineWidth(2)
ctx.stroke(CGRect(x: 1, y: 1, width: width - 2, height: height - 2))

// ── app icon ──────────────────────────────────────────────────────────────────
// Same construction as make-icon.swift, scaled onto a 120pt tile.
func drawIcon(in tile: CGRect) {
    let side = tile.width
    func s(_ v: CGFloat) -> CGFloat { v * side / 1024 }

    let squircle = CGPath(
        roundedRect: tile, cornerWidth: s(230), cornerHeight: s(230), transform: nil
    )
    ctx.saveGState()
    ctx.addPath(squircle)
    ctx.clip()
    let gradient = CGGradient(
        colorsSpace: sRGB, colors: [brandLight, brandDark] as CFArray, locations: [0, 1]
    )!
    ctx.drawLinearGradient(
        gradient,
        start: CGPoint(x: tile.minX, y: tile.maxY),
        end: CGPoint(x: tile.minX, y: tile.minY),
        options: []
    )
    ctx.restoreGState()

    let bodyWidth = s(560), bodyHeight = s(330)
    let body = CGRect(
        x: tile.midX - bodyWidth / 2,
        y: tile.midY - bodyHeight / 2 - s(40),
        width: bodyWidth, height: bodyHeight
    )
    let white = CGColor(red: 1, green: 1, blue: 1, alpha: 0.96)
    ctx.setFillColor(white)
    ctx.addPath(CGPath(roundedRect: body, cornerWidth: s(44), cornerHeight: s(44), transform: nil))
    ctx.fillPath()

    ctx.setStrokeColor(white)
    ctx.setLineWidth(s(52))
    ctx.setLineCap(.round)
    ctx.addArc(
        center: CGPoint(x: tile.midX, y: body.maxY), radius: s(120),
        startAngle: 0, endAngle: .pi, clockwise: false
    )
    ctx.strokePath()

    let latchWidth = s(150), latchHeight = s(70)
    let latch = CGRect(
        x: tile.midX - latchWidth / 2, y: body.midY - latchHeight / 2,
        width: latchWidth, height: latchHeight
    )
    ctx.setFillColor(brandDark)
    ctx.addPath(CGPath(roundedRect: latch, cornerWidth: s(18), cornerHeight: s(18), transform: nil))
    ctx.fillPath()
}

drawIcon(in: CGRect(x: 84, y: 428, width: 118, height: 118))

// ── text ──────────────────────────────────────────────────────────────────────
func draw(
    _ text: String, x: CGFloat, y: CGFloat,
    size: CGFloat, weight: NSFont.Weight, color: NSColor, tracking: CGFloat = 0
) {
    let attributes: [NSAttributedString.Key: Any] = [
        .font: NSFont.systemFont(ofSize: size, weight: weight),
        .foregroundColor: color,
        .kern: tracking,
    ]
    NSAttributedString(string: text, attributes: attributes)
        .draw(at: NSPoint(x: x, y: y))
}

let white = NSColor.white
let muted = NSColor(srgbRed: 0.60, green: 0.64, blue: 0.73, alpha: 1)
let faint = NSColor(srgbRed: 0.43, green: 0.47, blue: 0.58, alpha: 1)

draw("MACOS 14+  ·  UNIVERSAL  ·  NO NETWORK CODE",
     x: 236, y: 500, size: 19, weight: .semibold, color: faint, tracking: 2.4)
draw("Toolbox", x: 232, y: 424, size: 76, weight: .bold, color: white, tracking: -2)

draw("The small file jobs, done on your Mac.",
     x: 86, y: 322, size: 42, weight: .medium, color: white, tracking: -0.6)
draw("Unlock PDFs, convert HEIC, compress and resize images — in batches,",
     x: 86, y: 258, size: 26, weight: .regular, color: muted)
draw("entirely offline. Free and open source.",
     x: 86, y: 218, size: 26, weight: .regular, color: muted)

// ── tool chips ────────────────────────────────────────────────────────────────
let chips: [(String, CGColor)] = [
    ("Remove PDF Password", CGColor(red: 1.00, green: 0.62, blue: 0.04, alpha: 1)),
    ("Convert Image Format", CGColor(red: 0.04, green: 0.52, blue: 1.00, alpha: 1)),
    ("Compress Images", CGColor(red: 0.19, green: 0.82, blue: 0.35, alpha: 1)),
    ("Resize Images", CGColor(red: 0.75, green: 0.35, blue: 0.95, alpha: 1)),
]

var chipX: CGFloat = 86
let chipFont = NSFont.systemFont(ofSize: 20, weight: .medium)
for (label, tint) in chips {
    let textWidth = (label as NSString)
        .size(withAttributes: [.font: chipFont]).width
    let chipWidth = textWidth + 62
    let chip = CGRect(x: chipX, y: 104, width: chipWidth, height: 50)

    ctx.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.055))
    ctx.addPath(CGPath(roundedRect: chip, cornerWidth: 25, cornerHeight: 25, transform: nil))
    ctx.fillPath()
    ctx.setStrokeColor(tint.copy(alpha: 0.42)!)
    ctx.setLineWidth(1.5)
    ctx.addPath(CGPath(roundedRect: chip, cornerWidth: 25, cornerHeight: 25, transform: nil))
    ctx.strokePath()

    ctx.setFillColor(tint)
    ctx.fillEllipse(in: CGRect(x: chipX + 22, y: 124, width: 11, height: 11))

    draw(label, x: chipX + 44, y: 118, size: 20, weight: .medium, color: muted)
    chipX += chipWidth + 14
}

draw("github.com/lazzyms/toolbox", x: 86, y: 44, size: 20, weight: .regular, color: faint)

NSGraphicsContext.restoreGraphicsState()

// ── write ─────────────────────────────────────────────────────────────────────
try FileManager.default.createDirectory(
    at: outputDirectory, withIntermediateDirectories: true
)
let destination = outputDirectory.appendingPathComponent("og.png")

// Drawn at 2x for clean antialiasing, then downsampled to the 1200×630 that
// Open Graph actually asks for. Shipping the 2x bitmap would be ~2 MB for an
// image every share preview fetches.
guard let full = rep.cgImage else {
    FileHandle.standardError.write(Data("could not read back the render\n".utf8))
    exit(1)
}

// The card is fully opaque, so drop the alpha channel rather than paying for it
// in every byte of the PNG.
guard let output = CGContext(
    data: nil,
    width: width, height: height,
    bitsPerComponent: 8, bytesPerRow: 0,
    space: sRGB,
    bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
) else {
    FileHandle.standardError.write(Data("could not allocate the output bitmap\n".utf8))
    exit(1)
}
output.interpolationQuality = .high
output.draw(full, in: CGRect(x: 0, y: 0, width: width, height: height))

guard let scaled = output.makeImage() else {
    FileHandle.standardError.write(Data("downsample failed\n".utf8))
    exit(1)
}

guard let sink = CGImageDestinationCreateWithURL(
    destination as CFURL, "public.png" as CFString, 1, nil
) else {
    FileHandle.standardError.write(Data("could not open \(destination.path)\n".utf8))
    exit(1)
}
CGImageDestinationAddImage(sink, scaled, nil)
guard CGImageDestinationFinalize(sink) else {
    FileHandle.standardError.write(Data("PNG encoding failed\n".utf8))
    exit(1)
}

let bytes = (try? FileManager.default
    .attributesOfItem(atPath: destination.path)[.size] as? Int) ?? nil
let size = bytes.map { " — \($0 / 1024) KB" } ?? ""
print("Wrote \(destination.path) (\(width)×\(height))\(size)")
