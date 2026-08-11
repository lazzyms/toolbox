import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// Drag-and-drop target plus a Choose Files button.
struct DropZone: View {
    let prompt: String
    let allowedExtensions: Set<String>
    /// Nil allows any file type in the open panel; used for PDFs.
    let contentTypes: [UTType]
    @Binding var files: [URL]

    @State private var isTargeted = false

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: isTargeted ? "arrow.down.doc.fill" : "doc.badge.plus")
                .font(.system(size: 34, weight: .light))
                .foregroundStyle(isTargeted ? Color.accentColor : .secondary)
                .accessibilityHidden(true)

            Text(prompt)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)

            Button("Choose Files…", action: openPanel)
                .controlSize(.large)
        }
        .frame(maxWidth: .infinity)
        .padding(.vertical, 30)
        .background {
            RoundedRectangle(cornerRadius: 12)
                .fill(isTargeted ? Color.accentColor.opacity(0.08) : Color(nsColor: .textBackgroundColor).opacity(0.4))
        }
        .overlay {
            RoundedRectangle(cornerRadius: 12)
                .strokeBorder(
                    isTargeted ? Color.accentColor : Color.secondary.opacity(0.35),
                    style: StrokeStyle(lineWidth: isTargeted ? 2 : 1, dash: [6, 4])
                )
        }
        .animation(.easeOut(duration: 0.15), value: isTargeted)
        .onDrop(of: [.fileURL], isTargeted: $isTargeted, perform: handleDrop)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(prompt)
    }

    private func openPanel() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        if !contentTypes.isEmpty {
            panel.allowedContentTypes = contentTypes
        }
        guard panel.runModal() == .OK else { return }
        add(panel.urls)
    }

    private func handleDrop(_ providers: [NSItemProvider]) -> Bool {
        // Resolve every provider, then apply once — appending per-callback would
        // interleave and scramble the order of a multi-file drop.
        let group = DispatchGroup()
        let lock = NSLock()
        var resolved: [Int: URL] = [:]

        for (index, provider) in providers.enumerated() {
            group.enter()
            _ = provider.loadObject(ofClass: URL.self) { url, _ in
                defer { group.leave() }
                guard let url, url.isFileURL else { return }
                lock.lock()
                resolved[index] = url
                lock.unlock()
            }
        }

        group.notify(queue: .main) {
            add(resolved.sorted { $0.key < $1.key }.map(\.value))
        }
        return true
    }

    private func add(_ incoming: [URL]) {
        let accepted = incoming.filter { url in
            allowedExtensions.isEmpty
                || allowedExtensions.contains(url.pathExtension.lowercased())
        }
        var seen = Set(files.map(\.standardizedFileURL))
        for url in accepted where seen.insert(url.standardizedFileURL).inserted {
            files.append(url)
        }
    }
}
