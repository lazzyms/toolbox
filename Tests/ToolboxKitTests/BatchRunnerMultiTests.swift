import Foundation
import Testing
@testable import ToolboxKit

@Suite("BatchRunnerMulti")
struct BatchRunnerMultiTests {

    @Test("an outcome can report many outputs and output is the first")
    func manyOutputs() {
        let input = URL(fileURLWithPath: "/tmp/in.pdf")
        let first = URL(fileURLWithPath: "/tmp/in-1.jpg")
        let second = URL(fileURLWithPath: "/tmp/in-2.jpg")

        let outcome = JobOutcome(input: input, outputs: [first, second], detail: "2 pages")

        #expect(outcome.outputs == [first, second])
        #expect(outcome.output == first)
    }

    @Test("the single-output initialiser still routes to outputs")
    func singleOutputInitialiser() {
        let input = URL(fileURLWithPath: "/tmp/in.pdf")
        let output = URL(fileURLWithPath: "/tmp/out.pdf")

        let present = JobOutcome(input: input, output: output, detail: "")
        #expect(present.outputs == [output])
        #expect(present.output == output)

        let absent = JobOutcome(input: input, output: nil, detail: "")
        #expect(absent.outputs.isEmpty)
        #expect(absent.output == nil)
    }

    @Test("many inputs produce exactly one outcome, in their given order")
    func singleOutcomeForManyInputs() async {
        let inputs = (0..<4).map { URL(fileURLWithPath: "/tmp/part-\($0).pdf") }

        let outcome = await BatchRunner.runSingle(inputs) { received, consume in
            #expect(received == inputs)
            for _ in received { consume() }
            return JobOutcome(
                input: received[0],
                outputs: [URL(fileURLWithPath: "/tmp/merged.pdf")],
                detail: "Merged 4 pages"
            )
        }

        #expect(outcome.succeeded)
        #expect(outcome.outputs.count == 1)
        #expect(outcome.output?.lastPathComponent == "merged.pdf")
    }

    @Test("progress is reported once per input consumed, in order")
    func progressPerInputConsumed() async {
        let inputs = (0..<4).map { URL(fileURLWithPath: "/tmp/part-\($0).pdf") }
        let recorder = ProgressRecorder()

        let outcome = await BatchRunner.runSingle(
            inputs,
            progress: { done, total in recorder.record(done, total) }
        ) { received, consume in
            for _ in received { consume() }
            return JobOutcome(input: received[0], outputs: [], detail: "")
        }

        #expect(outcome.succeeded)
        #expect(recorder.values == [
            ProgressStep(done: 1, total: 4),
            ProgressStep(done: 2, total: 4),
            ProgressStep(done: 3, total: 4),
            ProgressStep(done: 4, total: 4),
        ])
    }

    @Test("consume never overreports past the input count")
    func progressIsCappedAtTotal() async {
        let inputs = (0..<3).map { URL(fileURLWithPath: "/tmp/part-\($0).pdf") }
        let recorder = ProgressRecorder()

        let outcome = await BatchRunner.runSingle(
            inputs,
            progress: { done, total in recorder.record(done, total) }
        ) { received, consume in
            for _ in 0..<5 { consume() }
            return JobOutcome(input: received[0], outputs: [], detail: "")
        }

        #expect(outcome.succeeded)
        #expect(recorder.values == [
            ProgressStep(done: 1, total: 3),
            ProgressStep(done: 2, total: 3),
            ProgressStep(done: 3, total: 3),
        ])
    }

    @Test("a throwing job becomes one failed outcome on the first input")
    func failureBecomesOneOutcome() async {
        let inputs = (0..<3).map { URL(fileURLWithPath: "/tmp/part-\($0).pdf") }

        let outcome = await BatchRunner.runSingle(inputs) { received, _ in
            throw ToolboxError.cannotOpen(received[1])
        }

        #expect(!outcome.succeeded)
        #expect(outcome.failure?.isEmpty == false)
        #expect(outcome.input == inputs[0])
        #expect(outcome.outputs.isEmpty)
        #expect(outcome.output == nil)
    }

    @Test("empty input reports a failure without running the job")
    func emptyInput() async {
        let outcome = await BatchRunner.runSingle([]) { _, _ in
            Issue.record("the job must not run for empty input")
            return JobOutcome(input: URL(fileURLWithPath: "/tmp/x"), outputs: [], detail: "")
        }

        #expect(!outcome.succeeded)
        #expect(outcome.failure != nil)
    }
}

/// One step of runSingle progress, as an Equatable value for assertions.
private struct ProgressStep: Equatable {
    let done: Int
    let total: Int
}

/// Thread-safe recorder for the runSingle progress assertions.
private final class ProgressRecorder: @unchecked Sendable {
    private let lock = NSLock()
    private var _values: [ProgressStep] = []

    var values: [ProgressStep] {
        lock.withLock { _values }
    }

    func record(_ done: Int, _ total: Int) {
        lock.withLock { _values.append(ProgressStep(done: done, total: total)) }
    }
}
