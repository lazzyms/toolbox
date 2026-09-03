import PDFKit
import SwiftUI
import ToolboxKit

/// The one tool that isn't drop-options-run: a single dropped PDF becomes a
/// grid of page thumbnails that can be reordered, rotated and deleted in place,
/// then saved as a new "-organized" copy.
///
/// The editing model lives here as plain data — an ordered array of entries,
/// each pointing at an original page index plus accumulated quarter turns —
/// which maps one-to-one onto `OrganizePlan`. The kit rebuilds the document;
/// this pane only decides what goes where.
///
/// Reordering ships as per-page move up/down buttons rather than drag-and-drop:
/// drag across a lazily-rendered grid needs drop delegates fighting the scroll
/// gesture on every cell, and buttons keep the whole plan legible in v1.
struct OrganizeView: View {
    let utility: Utility

    /// One slot in the output document. `pageIndex` is stable (it names an
    /// original page), while the entry's position in `pages` is where it lands.
    private struct PageEntry: Identifiable {
        let id = UUID()
        let pageIndex: Int
        var degrees: Int
    }

    @State private var files: [URL] = []
    @State private var loadedURL: URL?
    @State private var document: PDFDocument?
    @State private var pages: [PageEntry] = []
    @State private var thumbnails: [Int: NSImage] = [:]
    @State private var loadFailure: String?
    @State private var location: OutputLocation = .alongsideInput
    @State private var outcomes: [JobOutcome] = []
    @State private var isRunning = false

    @Environment(\.toolPresentation) private var presentation

    var body: some View {
        ToolScaffold(
            utility: utility,
            fileCount: files.isEmpty ? 0 : 1,
            isRunning: isRunning,
            progress: nil,
            runTitle: "Save Organized PDF",
            canRun: canRun,
            run: start
        ) {
            DestinationPicker(location: $location)
            Text("Rotate, delete or move pages, then save. The output is a new file — the original is never touched.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .padding(.leading, presentation.explanationInset)
                .fixedSize(horizontal: false, vertical: true)
        } content: {
            if let input = files.first {
                loadedHeader(for: input)
            } else {
                DropZone(
                    prompt: "Drop one PDF to organize",
                    allowedExtensions: ["pdf"],
                    contentTypes: [.pdf],
                    files: Binding(
                        get: { files },
                        set: { replaceInput(with: $0) }
                    )
                )
            }

            switch loadFailure {
            case .some(let message):
                Label(message, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                    .font(.callout)
            case .none:
                if document != nil, !pages.isEmpty {
                    pageGrid
                }
            }

            if !outcomes.isEmpty { ResultsList(outcomes: outcomes) }
        }
    }

    // MARK: - Pieces

    private func loadedHeader(for input: URL) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "doc.fill")
            Text(input.lastPathComponent)
                .lineLimit(1)
                .truncationMode(.middle)
            Text("\(pages.count) page\(pages.count == 1 ? "" : "s")")
                .font(.caption)
                .foregroundStyle(.secondary)
            Spacer()
            Button("Choose Another") { clearEditor() }
                .buttonStyle(.link)
        }
    }

    /// Bounded in its own scroll view: laziness needs a finite viewport, so a
    /// 300-page document materialises thumbnails only for the visible rows.
    private var pageGrid: some View {
        ScrollView {
            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: 150), spacing: 10)],
                spacing: 10
            ) {
                ForEach(pages) { entry in
                    pageCell(entry)
                }
            }
            .padding(2)
        }
        .frame(maxHeight: presentation.isCompact ? 260 : 430)
    }

    private func pageCell(_ entry: PageEntry) -> some View {
        let index = pages.firstIndex { $0.id == entry.id }

        return VStack(spacing: 6) {
            ZStack(alignment: .topTrailing) {
                Group {
                    if let image = thumbnails[entry.pageIndex] {
                        Image(nsImage: image)
                            .resizable()
                            .scaledToFit()
                    } else {
                        ProgressView()
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
                .frame(maxWidth: .infinity)
                .frame(height: 110)
                .rotationEffect(.degrees(Double(entry.degrees)))
                .animation(.easeOut(duration: 0.15), value: entry.degrees)

                Text("\(entry.pageIndex + 1)")
                    .font(.caption2.weight(.semibold).monospacedDigit())
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1)
                    .background(.quaternary, in: Capsule())
                    .help("Original page \(entry.pageIndex + 1)")
            }
            .frame(height: 116)

            HStack(spacing: 2) {
                cellButton("rotate.left", "Rotate left") {
                    turn(entry, by: -90)
                }
                cellButton("arrow.up", "Move earlier") {
                    move(entry: entry, delta: -1)
                }
                .disabled(index == nil || index! == 0)
                cellButton("arrow.down", "Move later") {
                    move(entry: entry, delta: 1)
                }
                .disabled(index == nil || index! == pages.count - 1)
                cellButton("rotate.right", "Rotate right") {
                    turn(entry, by: 90)
                }
                Spacer(minLength: 0)
                cellButton("trash", "Delete page") {
                    delete(entry)
                }
                .foregroundStyle(.red)
            }
        }
        .padding(8)
        .background(Color(nsColor: .textBackgroundColor).opacity(0.5), in: .rect(cornerRadius: 10))
        .overlay {
            RoundedRectangle(cornerRadius: 10)
                .strokeBorder(Color.secondary.opacity(0.25))
        }
        .task(id: entry.pageIndex) {
            await renderThumbnailIfNeeded(for: entry.pageIndex)
        }
    }

    private func cellButton(_ symbol: String, _ name: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .frame(width: 22, height: 18)
                .contentShape(Rectangle())
        }
        .buttonStyle(.borderless)
        .controlSize(.small)
        .accessibilityLabel(name)
    }

    // MARK: - Editing

    private func turn(_ entry: PageEntry, by delta: Int) {
        guard let index = pages.firstIndex(where: { $0.id == entry.id }) else { return }
        pages[index].degrees = ((pages[index].degrees + delta) % 360 + 360) % 360
    }

    private func move(entry: PageEntry, delta: Int) {
        guard let index = pages.firstIndex(where: { $0.id == entry.id }) else { return }
        let target = index + delta
        guard pages.indices.contains(target) else { return }
        withAnimation(.easeOut(duration: 0.15)) {
            pages.swapAt(index, target)
        }
    }

    private func delete(_ entry: PageEntry) {
        withAnimation(.easeOut(duration: 0.15)) {
            pages.removeAll { $0.id == entry.id }
        }
    }

    // MARK: - Loading

    /// The only two ways `files` changes: a fresh drop (kept to one document —
    /// batching is meaningless for a hand-arranged file) and "Choose Another".
    /// Driving loading from the setter instead of `onChange` keeps the reset
    /// and the parse in the same transaction, with no reload loops.
    private func replaceInput(with candidates: [URL]) {
        let picked = Array(candidates.prefix(1))
        guard picked.first?.standardizedFileURL != loadedURL else { return }
        files = picked
        if let url = picked.first {
            loadDocument(from: url)
        } else {
            clearEditor()
        }
    }

    /// Opens on the main actor deliberately: `PDFDocument` parsing is lazy and
    /// fast — it's the *rendering* that costs, and that happens per visible
    /// cell below. Passing the document across actors would mean shipping
    /// non-Sendable PDFKit types through `Task`s for no measurable gain.
    private func loadDocument(from url: URL) {
        clearEditorState()
        loadedURL = url

        guard let doc = PDFDocument(url: url) else {
            loadFailure = "“\(url.lastPathComponent)” isn't a readable PDF."
            return
        }
        guard !doc.isLocked else {
            loadFailure = "“\(url.lastPathComponent)” is password-protected — remove the password first."
            return
        }
        document = doc
        pages = (0..<doc.pageCount).map { PageEntry(pageIndex: $0, degrees: 0) }
    }

    private func clearEditorState() {
        document = nil
        pages = []
        thumbnails = [:]
        loadFailure = nil
    }

    private func clearEditor() {
        clearEditorState()
        loadedURL = nil
        files.removeAll()
    }

    /// Rendered once per original page and cached by index, so rotating or
    /// moving a page never re-renders it — the grid just re-applies a transform.
    /// `LazyVGrid` calls this only for cells that scroll into view.
    private func renderThumbnailIfNeeded(for pageIndex: Int) async {
        guard thumbnails[pageIndex] == nil,
              let page = document?.page(at: pageIndex)
        else { return }
        thumbnails[pageIndex] = page.thumbnail(of: CGSize(width: 320, height: 440), for: .mediaBox)
    }

    // MARK: - Running

    private var canRun: Bool {
        files.first != nil
            && document != nil
            && !pages.isEmpty
            && loadFailure == nil
            && !isRunning
    }

    private var plan: OrganizePlan {
        OrganizePlan(pages.map { entry in
            entry.degrees == 0
                ? .keep(index: entry.pageIndex)
                : .rotate(index: entry.pageIndex, degrees: entry.degrees)
        })
    }

    private func start() {
        guard canRun, let input = files.first else { return }
        let plan = plan
        let destination = location

        isRunning = true
        outcomes = []

        Task {
            let outcome = await Task.detached(priority: .userInitiated) {
                Self.organizeJob(plan: plan, input: input, location: destination)
            }.value

            await MainActor.run {
                outcomes = [outcome]
                isRunning = false
                if outcome.succeeded {
                    clearEditor()
                }
            }
        }
    }

    /// One document → one output, so like GIF creation this bypasses
    /// `BatchRunner`'s per-file model and reports as a single outcome. Runs
    /// detached because rebuilding and writing the PDF must not hold the main
    /// actor; everything captured (`plan`, URLs, location) is `Sendable`.
    nonisolated static func organizeJob(
        plan: OrganizePlan, input: URL, location: OutputLocation
    ) -> JobOutcome {
        do {
            let output = try PageOrganizer.apply(plan: plan, to: input, location: location)
            let size = ByteFormat.string(OutputNaming.fileSize(of: output))
            return JobOutcome(
                input: input,
                output: output,
                detail: "\(plan.ops.count) page\(plan.ops.count == 1 ? "" : "s") · \(size)"
            )
        } catch {
            return JobOutcome(
                input: input,
                output: nil,
                detail: "",
                failure: (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
            )
        }
    }
}
