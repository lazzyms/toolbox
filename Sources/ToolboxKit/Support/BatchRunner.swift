import Foundation

/// Outcome for a single file in a batch.
public struct JobOutcome: Sendable, Identifiable {
    public let id: URL
    public let input: URL
    /// Every file a job produced. Most tools produce one, which is why
    /// `output` — the first element — has always been the public face. Split,
    /// extract-pages and PDF→JPG write several and report them all here.
    public let outputs: [URL]
    public let detail: String
    public let failure: String?

    public var succeeded: Bool { failure == nil }

    /// The first output, or nil when the job produced nothing. Existing call
    /// sites and tests keep reading this; tools that write one file per input
    /// never need to look at `outputs` at all.
    public var output: URL? { outputs.first }

    public init(input: URL, outputs: [URL], detail: String, failure: String? = nil) {
        self.id = input
        self.input = input
        self.outputs = outputs
        self.detail = detail
        self.failure = failure
    }

    public init(input: URL, output: URL?, detail: String, failure: String? = nil) {
        self.init(input: input, outputs: output.map { [$0] } ?? [], detail: detail, failure: failure)
    }
}

/// Runs a per-file job across many files with bounded parallelism.
///
/// Image encoding is CPU-bound, so the cap is the core count — going wider just
/// thrashes memory on large batches. One file failing never stops the rest.
public enum BatchRunner {

    public static func run(
        _ inputs: [URL],
        maxConcurrent: Int = max(1, ProcessInfo.processInfo.activeProcessorCount),
        progress: @Sendable @escaping (Int, Int) -> Void = { _, _ in },
        job: @Sendable @escaping (URL) throws -> JobOutcome
    ) async -> [JobOutcome] {
        guard !inputs.isEmpty else { return [] }

        let total = inputs.count
        var results: [JobOutcome] = []
        results.reserveCapacity(total)
        var completed = 0

        await withTaskGroup(of: JobOutcome.self) { group in
            var next = 0
            let limit = min(maxConcurrent, total)

            func addTask(_ input: URL) {
                group.addTask {
                    do {
                        return try job(input)
                    } catch {
                        let message = (error as? LocalizedError)?.errorDescription
                            ?? error.localizedDescription
                        return JobOutcome(
                            input: input, output: nil, detail: "", failure: message
                        )
                    }
                }
            }

            while next < limit {
                addTask(inputs[next])
                next += 1
            }

            // Refill as each slot frees so the pool stays saturated instead of
            // waiting on the slowest file in each wave.
            while let outcome = await group.next() {
                results.append(outcome)
                completed += 1
                progress(completed, total)
                if next < total {
                    addTask(inputs[next])
                    next += 1
                }
            }
        }

        // Restore the user's original ordering; completion order is arbitrary.
        let order = Dictionary(uniqueKeysWithValues: inputs.enumerated().map { ($1, $0) })
        return results.sorted { (order[$0.input] ?? 0) < (order[$1.input] ?? 0) }
    }
}
