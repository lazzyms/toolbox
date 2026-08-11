import Foundation

/// Where a job should write its result.
public enum OutputLocation: Sendable, Hashable {
    /// Beside the input file.
    case alongsideInput
    /// Into a specific directory the user picked.
    case directory(URL)

    func directory(forInput input: URL) -> URL {
        switch self {
        case .alongsideInput: return input.deletingLastPathComponent()
        case .directory(let url): return url
        }
    }
}

public enum OutputNaming {
    /// Builds a destination URL, never overwriting an existing file.
    ///
    /// `photo.heic` → `photo.png`, and if that exists → `photo-1.png`, `photo-2.png`, …
    /// The suffix is applied before the collision counter so names stay readable.
    public static func destination(
        for input: URL,
        in location: OutputLocation,
        suffix: String = "",
        extension ext: String,
        fileManager: FileManager = .default
    ) -> URL {
        let dir = location.directory(forInput: input)
        let base = input.deletingPathExtension().lastPathComponent + suffix
        var candidate = dir.appendingPathComponent(base).appendingPathExtension(ext)

        var counter = 1
        while fileManager.fileExists(atPath: candidate.path) {
            candidate = dir
                .appendingPathComponent("\(base)-\(counter)")
                .appendingPathExtension(ext)
            counter += 1
            // Defensive: a pathological directory shouldn't spin forever.
            if counter > 9999 { break }
        }
        return candidate
    }

    public static func fileSize(of url: URL) -> Int64 {
        let values = try? url.resourceValues(forKeys: [.fileSizeKey])
        return Int64(values?.fileSize ?? 0)
    }
}
