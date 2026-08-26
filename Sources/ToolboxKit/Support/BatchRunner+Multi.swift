import Foundation

extension BatchRunner {

    /// Runs one job across the whole input list, reporting a single outcome.
    ///
    /// This is the entry point for the many→one tools (merge): N inputs become
    /// one output, so there is no per-file outcome to report and per-file
    /// parallelism is meaningless. The job receives the inputs in their given
    /// order — page order is the whole point of merge — and reports progress by
    /// calling `consume` once per input as it processes them, so the progress
    /// bar in `ToolScaffold` counts the same unit it counts for per-file tools.
    public static func runSingle(
        _ inputs: [URL],
        progress: @Sendable @escaping (Int, Int) -> Void = { _, _ in },
        job: @Sendable @escaping ([URL], _ consume: @Sendable () -> Void) throws -> JobOutcome
    ) async -> JobOutcome {
        let total = inputs.count
        guard total > 0 else {
            return JobOutcome(
                input: URL(fileURLWithPath: ""),
                outputs: [],
                detail: "",
                failure: "Nothing to run"
            )
        }

        let counter = InputCounter(total: total, progress: progress)
        do {
            return try job(inputs) {
                counter.consume()
            }
        } catch {
            let message = (error as? LocalizedError)?.errorDescription
                ?? error.localizedDescription
            return JobOutcome(input: inputs[0], outputs: [], detail: "", failure: message)
        }
    }
}

/// Progress backing for `runSingle`. The job is one task, but `consume` is
/// handed out as a closure the framework can't reason about, so the shared
/// counter is locked to keep the reported count monotonic and capped at total.
private final class InputCounter: @unchecked Sendable {
    private let lock = NSLock()
    private let total: Int
    private let progress: @Sendable (Int, Int) -> Void
    private var done = 0

    init(total: Int, progress: @escaping @Sendable (Int, Int) -> Void) {
        self.total = total
        self.progress = progress
    }

    func consume() {
        var shouldReport = false
        var updated = 0
        lock.withLock {
            if done < total {
                done += 1
                updated = done
                shouldReport = true
            }
        }
        // One report per input actually consumed; a job that over-consumes
        // just stops reporting rather than pushing the bar past 100%.
        if shouldReport {
            progress(updated, total)
        }
    }
}
