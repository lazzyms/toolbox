import CoreGraphics
import CoreText

public enum StampSize: Sendable {
    case points(CGFloat)
    case fraction(CGFloat)
}

public enum StampAnchor: Sendable {
    case topLeft
    case top
    case topRight
    case left
    case center
    case right
    case bottomLeft
    case bottom
    case bottomRight
    case tiled(spacing: Double)
}

public struct TextStamp: Sendable {
    public var text: String
    public var fontName: String
    public var size: StampSize
    public var color: CGColor
    public var opacity: Double
    public var rotationDegrees: Double

    public init(
        text: String,
        fontName: String = "Helvetica",
        size: StampSize = .points(24),
        color: CGColor = CGColor.black,
        opacity: Double = 1.0,
        rotationDegrees: Double = 0
    ) {
        self.text = text
        self.fontName = fontName
        self.size = size
        self.color = color
        self.opacity = opacity
        self.rotationDegrees = rotationDegrees
    }
}

public enum GraphicsStamp {
    public static func draw(
        _ stamp: TextStamp,
        in context: CGContext,
        bounds: CGRect,
        anchor: StampAnchor,
        inset: Double = 0
    ) {
        guard !stamp.text.isEmpty else { return }
        context.saveGState()
        defer { context.restoreGState() }

        context.setAlpha(stamp.opacity)

        let fontSize = resolvedFontSize(for: stamp.size, boundsWidth: bounds.width)
        let font = CTFontCreateWithName(stamp.fontName as CFString, fontSize, nil)
        let attributes: [CFString: Any] = [
            kCTFontAttributeName: font,
            kCTForegroundColorAttributeName: stamp.color
        ]
        guard let attributed = CFAttributedStringCreate(nil, stamp.text as CFString, attributes as CFDictionary) else { return }
        let line = CTLineCreateWithAttributedString(attributed)
        var ascent: CGFloat = 0
        var descent: CGFloat = 0
        var leading: CGFloat = 0
        CTLineGetTypographicBounds(line, &ascent, &descent, &leading)
        let textHeight = ascent + descent
        let textWidth = CGFloat(CTLineGetTypographicBounds(line, nil, nil, nil))
        // Use actual width from line
        var lineWidth = CGFloat(0)
        let lineBounds = CTLineGetImageBounds(line, context)
        lineWidth = lineBounds.width

        let origin = anchorOrigin(
            for: anchor,
            bounds: bounds,
            textWidth: lineWidth,
            textHeight: textHeight,
            inset: inset
        )

        context.translateBy(x: origin.x, y: origin.y)
        if stamp.rotationDegrees != 0 {
            let rad = stamp.rotationDegrees * .pi / 180
            context.rotate(by: rad)
        }
        // CoreGraphics origin is bottom-left, CoreText expects baseline
        context.textPosition = CGPoint(x: 0, y: -descent)
        // Adjust for height? The translate already offset.
        // Actually we want baseline at origin.y
        // Let's move text baseline to origin
        context.translateBy(x: 0, y: textHeight)
        CTLineDraw(line, context)
    }

    public static func draw(
        _ image: CGImage,
        in context: CGContext,
        bounds: CGRect,
        anchor: StampAnchor,
        scale: Double = 1.0,
        opacity: Double = 1.0
    ) {
        context.saveGState()
        defer { context.restoreGState() }
        context.setAlpha(opacity)
        let imgSize = CGSize(width: CGFloat(image.width) * CGFloat(scale), height: CGFloat(image.height) * CGFloat(scale))
        let origin = anchorOrigin(
            for: anchor,
            bounds: bounds,
            textWidth: imgSize.width,
            textHeight: imgSize.height,
            inset: 0
        )
        let rect = CGRect(origin: origin, size: imgSize)
        context.draw(image, in: rect)
    }

    private static func resolvedFontSize(for size: StampSize, boundsWidth: CGFloat) -> CGFloat {
        switch size {
        case .points(let pt): return pt
        case .fraction(let f): return max(1, boundsWidth * CGFloat(f))
        }
    }

    private static func anchorOrigin(
        for anchor: StampAnchor,
        bounds: CGRect,
        textWidth: CGFloat,
        textHeight: CGFloat,
        inset: Double
    ) -> CGPoint {
        let insetPt = CGFloat(inset)
        let box = bounds.insetBy(dx: insetPt, dy: insetPt)

        switch anchor {
        case .topLeft:
            return CGPoint(x: box.minX, y: box.maxY)
        case .top:
            return CGPoint(x: box.midX - textWidth / 2, y: box.maxY)
        case .topRight:
            return CGPoint(x: box.maxX - textWidth, y: box.maxY)
        case .left:
            return CGPoint(x: box.minX, y: box.midY - textHeight / 2)
        case .center:
            return CGPoint(x: box.midX - textWidth / 2, y: box.midY - textHeight / 2 + textHeight / 2)
        case .right:
            return CGPoint(x: box.maxX - textWidth, y: box.midY - textHeight / 2)
        case .bottomLeft:
            return CGPoint(x: box.minX, y: box.minY)
        case .bottom:
            return CGPoint(x: box.midX - textWidth / 2, y: box.minY)
        case .bottomRight:
            return CGPoint(x: box.maxX - textWidth, y: box.minY)
        case .tiled:
            // For tiled we will be called per tile; caller handles positioning
            return CGPoint(x: box.minX, y: box.minY)
        }
    }
}
