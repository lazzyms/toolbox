import Foundation

public enum ByteFormat {
    public static func string(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    /// "2.4 MB → 810 KB (66% smaller)"
    public static func savings(from original: Int64, to new: Int64) -> String {
        let base = "\(string(original)) → \(string(new))"
        guard original > 0, new > 0 else { return base }
        let delta = Double(original - new) / Double(original)
        let pct = abs(Int((delta * 100).rounded()))
        if pct == 0 { return "\(base) (about the same)" }
        return delta > 0 ? "\(base) (\(pct)% smaller)" : "\(base) (\(pct)% larger)"
    }
}
