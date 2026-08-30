import AppKit
import SwiftUI

/// Renders one of the locally bundled Tabler outline icons.
///
/// Keeping the SVGs in the app bundle makes the sidebar deterministic and
/// avoids adding a runtime dependency just for presentation assets.
struct TablerIcon: View {
    let name: String
    let color: Color

    var body: some View {
        if let image = loadImage() {
            Image(nsImage: image)
                .renderingMode(.template)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .foregroundStyle(color)
        } else {
            Image(systemName: "questionmark.square.dashed")
                .foregroundStyle(color)
        }
    }

    private func loadImage() -> NSImage? {
        guard let url = Bundle.module.url(
            forResource: name,
            withExtension: "svg",
            subdirectory: "Tabler"
        ), let image = NSImage(contentsOf: url) else {
            assertionFailure("Missing bundled Tabler icon: \(name)")
            return nil
        }
        image.isTemplate = true
        return image
    }
}
