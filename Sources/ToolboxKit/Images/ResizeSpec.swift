import CoreGraphics
import Foundation

/// How to compute a new pixel size from the original.
public enum ResizeSpec: Sendable, Equatable {
    case none
    /// Scale so the image fits inside the box, preserving aspect ratio.
    /// A nil side is unconstrained.
    case fit(width: Int?, height: Int?)
    /// Exact pixel size. Distorts unless the ratio happens to match.
    case exact(width: Int, height: Int)
    /// Percentage of the original, e.g. 50 → half size.
    case percent(Double)
    /// Longest side clamped to this many pixels, ratio preserved.
    case longestSide(Int)

    /// Returns the target size, or nil when no resize is needed.
    /// Never upscales beyond the requested box for `.fit`/`.longestSide`
    /// when `allowUpscale` is false.
    public func target(for source: CGSize, allowUpscale: Bool = false) -> CGSize? {
        guard source.width > 0, source.height > 0 else { return nil }

        let computed: CGSize?
        switch self {
        case .none:
            return nil

        case .fit(let w, let h):
            let maxW = w.map(Double.init)
            let maxH = h.map(Double.init)
            guard maxW != nil || maxH != nil else { return nil }
            let scaleW = maxW.map { $0 / source.width } ?? .greatestFiniteMagnitude
            let scaleH = maxH.map { $0 / source.height } ?? .greatestFiniteMagnitude
            var scale = min(scaleW, scaleH)
            if !allowUpscale { scale = min(scale, 1.0) }
            computed = CGSize(width: source.width * scale, height: source.height * scale)

        case .exact(let w, let h):
            guard w > 0, h > 0 else { return nil }
            computed = CGSize(width: Double(w), height: Double(h))

        case .percent(let pct):
            guard pct > 0 else { return nil }
            let scale = pct / 100.0
            computed = CGSize(width: source.width * scale, height: source.height * scale)

        case .longestSide(let side):
            guard side > 0 else { return nil }
            let longest = max(source.width, source.height)
            var scale = Double(side) / longest
            if !allowUpscale { scale = min(scale, 1.0) }
            computed = CGSize(width: source.width * scale, height: source.height * scale)
        }

        guard let computed else { return nil }
        // Round to whole pixels and never collapse to zero.
        let rounded = CGSize(
            width: max(1, computed.width.rounded()),
            height: max(1, computed.height.rounded())
        )
        return rounded == source ? nil : rounded
    }

    public var isActive: Bool {
        switch self {
        case .none: return false
        case .fit(let w, let h): return (w ?? 0) > 0 || (h ?? 0) > 0
        case .exact(let w, let h): return w > 0 && h > 0
        case .percent(let p): return p > 0 && p != 100
        case .longestSide(let s): return s > 0
        }
    }
}
