#!/usr/bin/env swift

// Generates Resources/AppIcon.icns — a rounded-square "toolbox" mark.
// Committing a generator instead of a binary keeps the repo diffable and lets
// the icon be tweaked without a design tool.

import AppKit
import CoreGraphics
import Foundation

let outputDirectory = CommandLine.arguments.count > 1
    ? URL(fileURLWithPath: CommandLine.arguments[1])
    : URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
        .appendingPathComponent("Resources")

func drawIcon(side: CGFloat) -> CGImage {
    let context = CGContext(
        data: nil,
        width: Int(side), height: Int(side),
        bitsPerComponent: 8, bytesPerRow: 0,
        space: CGColorSpace(name: CGColorSpace.sRGB)!,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!

    let scale = side / 1024.0
    func s(_ value: CGFloat) -> CGFloat { value * scale }

    // Rounded-rect background with a vertical gradient, matching macOS icon shape.
    let inset = s(64)
    let rect = CGRect(x: inset, y: inset, width: side - inset * 2, height: side - inset * 2)
    let squircle = CGPath(
        roundedRect: rect, cornerWidth: s(200), cornerHeight: s(200), transform: nil
    )

    context.saveGState()
    context.addPath(squircle)
    context.clip()
    let gradient = CGGradient(
        colorsSpace: CGColorSpace(name: CGColorSpace.sRGB)!,
        colors: [
            CGColor(red: 0.29, green: 0.56, blue: 0.98, alpha: 1),
            CGColor(red: 0.16, green: 0.31, blue: 0.83, alpha: 1),
        ] as CFArray,
        locations: [0, 1]
    )!
    context.drawLinearGradient(
        gradient,
        start: CGPoint(x: 0, y: side),
        end: CGPoint(x: 0, y: 0),
        options: []
    )
    context.restoreGState()

    // Toolbox body.
    let bodyWidth = s(560), bodyHeight = s(330)
    let body = CGRect(
        x: (side - bodyWidth) / 2,
        y: (side - bodyHeight) / 2 - s(40),
        width: bodyWidth, height: bodyHeight
    )
    context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.96))
    context.addPath(CGPath(
        roundedRect: body, cornerWidth: s(44), cornerHeight: s(44), transform: nil
    ))
    context.fillPath()

    // Handle arc above the body.
    context.setStrokeColor(CGColor(red: 1, green: 1, blue: 1, alpha: 0.96))
    context.setLineWidth(s(52))
    context.setLineCap(.round)
    let handleRadius = s(120)
    context.addArc(
        center: CGPoint(x: side / 2, y: body.maxY),
        radius: handleRadius,
        startAngle: 0, endAngle: .pi, clockwise: false
    )
    context.strokePath()

    // Latch across the middle of the body.
    let latchWidth = s(150), latchHeight = s(70)
    let latch = CGRect(
        x: (side - latchWidth) / 2,
        y: body.midY - latchHeight / 2,
        width: latchWidth, height: latchHeight
    )
    context.setFillColor(CGColor(red: 0.16, green: 0.31, blue: 0.83, alpha: 1))
    context.addPath(CGPath(
        roundedRect: latch, cornerWidth: s(18), cornerHeight: s(18), transform: nil
    ))
    context.fillPath()

    return context.makeImage()!
}

// .icns needs each size at 1x and 2x.
let iconsetDirectory = URL(fileURLWithPath: NSTemporaryDirectory())
    .appendingPathComponent("AppIcon-\(UUID().uuidString).iconset")
try FileManager.default.createDirectory(
    at: iconsetDirectory, withIntermediateDirectories: true
)

for base in [16, 32, 128, 256, 512] {
    for scaleFactor in [1, 2] {
        let pixels = base * scaleFactor
        let name = scaleFactor == 1
            ? "icon_\(base)x\(base).png"
            : "icon_\(base)x\(base)@2x.png"
        let image = drawIcon(side: CGFloat(pixels))
        let url = iconsetDirectory.appendingPathComponent(name)
        let destination = CGImageDestinationCreateWithURL(
            url as CFURL, "public.png" as CFString, 1, nil
        )!
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            FileHandle.standardError.write(Data("failed writing \(name)\n".utf8))
            exit(1)
        }
    }
}

try FileManager.default.createDirectory(
    at: outputDirectory, withIntermediateDirectories: true
)
let icns = outputDirectory.appendingPathComponent("AppIcon.icns")

let process = Process()
process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
process.arguments = ["-c", "icns", iconsetDirectory.path, "-o", icns.path]
try process.run()
process.waitUntilExit()
try? FileManager.default.removeItem(at: iconsetDirectory)

guard process.terminationStatus == 0 else {
    FileHandle.standardError.write(Data("iconutil failed\n".utf8))
    exit(1)
}
print("Wrote \(icns.path)")
