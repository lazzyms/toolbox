#!/usr/bin/env swift

// Generates the instructional background shown in the mounted DMG window.
// Keeping it as source makes the Finder-facing install guidance easy to update.
//
//   swift Scripts/make-dmg-background.swift <output-path>

import AppKit
import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 2 else {
    FileHandle.standardError.write(Data("usage: make-dmg-background.swift <output-path>\n".utf8))
    exit(1)
}

let outputURL = URL(fileURLWithPath: CommandLine.arguments[1])
let width = 600
let height = 420
let scale = 2

guard let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: width * scale,
    pixelsHigh: height * scale,
    bitsPerSample: 8,
    samplesPerPixel: 4,
    hasAlpha: true,
    isPlanar: false,
    colorSpaceName: .deviceRGB,
    bytesPerRow: 0,
    bitsPerPixel: 0
) else {
    FileHandle.standardError.write(Data("could not allocate bitmap\n".utf8))
    exit(1)
}
rep.size = NSSize(width: width, height: height)

guard let graphicsContext = NSGraphicsContext(bitmapImageRep: rep) else {
    FileHandle.standardError.write(Data("could not make drawing context\n".utf8))
    exit(1)
}

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = graphicsContext
let context = graphicsContext.cgContext

let background = NSColor(srgbRed: 0.965, green: 0.969, blue: 0.976, alpha: 1)
background.setFill()
context.fill(CGRect(x: 0, y: 0, width: width, height: height))

let card = CGRect(x: 42, y: 298, width: 516, height: 104)
NSColor.white.setFill()
context.addPath(CGPath(roundedRect: card, cornerWidth: 16, cornerHeight: 16, transform: nil))
context.fillPath()
NSColor(srgbRed: 0.82, green: 0.84, blue: 0.88, alpha: 1).setStroke()
context.setLineWidth(1)
context.addPath(CGPath(roundedRect: card, cornerWidth: 16, cornerHeight: 16, transform: nil))
context.strokePath()

func draw(_ text: String, at point: CGPoint, size: CGFloat, weight: NSFont.Weight, color: NSColor) {
    NSAttributedString(
        string: text,
        attributes: [
            .font: NSFont.systemFont(ofSize: size, weight: weight),
            .foregroundColor: color,
        ]
    ).draw(at: NSPoint(x: point.x, y: point.y))
}

let ink = NSColor(srgbRed: 0.10, green: 0.12, blue: 0.16, alpha: 1)
let muted = NSColor(srgbRed: 0.32, green: 0.36, blue: 0.43, alpha: 1)
let blue = NSColor(srgbRed: 0.12, green: 0.35, blue: 0.84, alpha: 1)

draw("FIRST LAUNCH BLOCKED?", at: CGPoint(x: 70, y: 376), size: 16, weight: .bold, color: blue)
draw("Open System Settings → Privacy & Security", at: CGPoint(x: 70, y: 349), size: 15, weight: .semibold, color: ink)
draw("Scroll down to the “Toolbox” message and click “Open Anyway”.", at: CGPoint(x: 70, y: 326), size: 12, weight: .regular, color: muted)
draw("Then confirm the prompt. You only need this once.", at: CGPoint(x: 70, y: 307), size: 12, weight: .regular, color: muted)

NSGraphicsContext.restoreGraphicsState()

guard let png = rep.representation(using: .png, properties: [:]) else {
    FileHandle.standardError.write(Data("could not encode PNG\n".utf8))
    exit(1)
}
try png.write(to: outputURL)
