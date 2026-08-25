import SwiftUI

extension Utility {
    /// The image tools, in the order they appear in the sidebar.
    ///
    /// One entry per line, deliberately past the usual column budget: adding a
    /// tool is then a single appended line, and two branches that both add one
    /// conflict on that line alone — resolved by keeping both.
    static let imageTools: [Utility] = [
        Utility(id: "heic-convert", title: "Convert Image Format", shortTitle: "Convert", blurb: "HEIC to PNG, JPEG and back — batch friendly.", symbol: "arrow.triangle.2.circlepath", tint: .blue, category: .images, pane: { ConvertView(utility: $0) }),
        Utility(id: "compress", title: "Compress Images", shortTitle: "Compress", blurb: "Shrink files losslessly, or trade quality for size.", symbol: "arrow.down.circle.fill", tint: .green, category: .images, pane: { CompressView(utility: $0) }),
        Utility(id: "resize", title: "Resize Images", shortTitle: "Resize", blurb: "Scale by pixels, percentage or longest side.", symbol: "aspectratio.fill", tint: .purple, category: .images, pane: { ResizeView(utility: $0) }),
        Utility(id: "rotate", title: "Rotate & Flip Images", shortTitle: "Rotate", blurb: "Quarter turns and mirrors, without resampling — batch friendly.", symbol: "rotate.right", tint: .pink, category: .images, pane: { RotateView(utility: $0) }),
        Utility(id: "crop", title: "Crop Images", shortTitle: "Crop", blurb: "Cut to an anchored aspect ratio or a fixed pixel rectangle, in a batch.", symbol: "crop", tint: .indigo, category: .images, pane: { CropView(utility: $0) }),
        Utility(id: "icon-set", title: "Generate App Icons", shortTitle: "Icons", blurb: "Turn one image into a complete macOS, favicon, iOS or Android icon set.", symbol: "app.badge", tint: .orange, category: .images, pane: { IconSetView(utility: $0) }),
        Utility(id: "gif-create", title: "Create GIF", shortTitle: "GIF Maker", blurb: "Animate a batch of still images into a looped GIF.", symbol: "film.stack", tint: .teal, category: .images, pane: { GIFView(utility: $0, mode: .create) }),
        Utility(id: "gif-extract", title: "Extract GIF Frames", shortTitle: "Frames", blurb: "Split an animated GIF into its individual frames.", symbol: "square.stack.3d.up", tint: .teal, category: .images, pane: { GIFView(utility: $0, mode: .extract) }),
        Utility(id: "image-watermark", title: "Watermark Images", shortTitle: "Watermark", blurb: "Stamp text or a logo across a batch of images.", symbol: "drop.halffull", tint: .cyan, category: .images, pane: { ImageWatermarkView(utility: $0) }),
        Utility(id: "image-metadata", title: "Image Metadata", shortTitle: "Metadata", blurb: "See what a photo leaks, then strip EXIF and GPS without recompressing it.", symbol: "info.circle", tint: .mint, category: .images, pane: { ImageMetadataView(utility: $0) }),
        Utility(id: "image-tone", title: "Colour & Tone Adjustments", shortTitle: "Tone", blurb: "Batch brightness, contrast, saturation, exposure and one-tap presets.", symbol: "slider.horizontal.3", tint: .yellow, category: .images, pane: { ImageToneView(utility: $0) }),
        Utility(id: "tiff-pages", title: "Split & Combine TIFF", shortTitle: "TIFF Pages", blurb: "Break a multi-page TIFF into images, or bind a batch of images into one.", symbol: "rectangle.split.2x1", tint: .brown, category: .images, pane: { TIFFView(utility: $0) }),
    ]
}
