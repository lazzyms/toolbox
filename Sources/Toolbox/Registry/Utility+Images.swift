import SwiftUI

extension Utility {
    /// The image tools, in the order they appear in the sidebar.
    ///
    /// One entry per line, deliberately past the usual column budget: adding a
    /// tool is then a single appended line, and two branches that both add one
    /// conflict on that line alone — resolved by keeping both.
    static let imageTools: [Utility] = [
        Utility(id: "heic-convert", title: "Convert Image Format", shortTitle: "Convert", blurb: "HEIC to PNG, JPEG and back — batch friendly.", category: .images, pane: { ConvertView(utility: $0) }),
        Utility(id: "compress", title: "Compress Images", shortTitle: "Compress", blurb: "Shrink files losslessly, or trade quality for size.", category: .images, pane: { CompressView(utility: $0) }),
        Utility(id: "resize", title: "Resize Images", shortTitle: "Resize", blurb: "Scale by pixels, percentage or longest side.", category: .images, pane: { ResizeView(utility: $0) }),
        Utility(id: "rotate", title: "Rotate & Flip Images", shortTitle: "Rotate", blurb: "Quarter turns and mirrors, without resampling — batch friendly.", category: .images, pane: { RotateView(utility: $0) }),
        Utility(id: "crop", title: "Crop Images", shortTitle: "Crop", blurb: "Cut to an anchored aspect ratio or a fixed pixel rectangle, in a batch.", category: .images, pane: { CropView(utility: $0) }),
        Utility(id: "icon-set", title: "Generate App Icons", shortTitle: "Icons", blurb: "Turn one image into a complete macOS, favicon, iOS or Android icon set.", category: .images, pane: { IconSetView(utility: $0) }),
        Utility(id: "gif-create", title: "Create GIF", shortTitle: "GIF Maker", blurb: "Animate a batch of still images into a looped GIF.", category: .images, pane: { GIFView(utility: $0, mode: .create) }),
        Utility(id: "gif-extract", title: "Extract GIF Frames", shortTitle: "Frames", blurb: "Split an animated GIF into its individual frames.", category: .images, pane: { GIFView(utility: $0, mode: .extract) }),
        Utility(id: "image-watermark", title: "Watermark Images", shortTitle: "Watermark", blurb: "Stamp text or a logo across a batch of images.", category: .images, pane: { ImageWatermarkView(utility: $0) }),
        Utility(id: "image-metadata", title: "Image Metadata", shortTitle: "Metadata", blurb: "See what a photo leaks, then strip EXIF and GPS without recompressing it.", category: .images, pane: { ImageMetadataView(utility: $0) }),
        Utility(id: "image-tone", title: "Colour & Tone Adjustments", shortTitle: "Tone", blurb: "Batch brightness, contrast, saturation, exposure and one-tap presets.", category: .images, pane: { ImageToneView(utility: $0) }),
        Utility(id: "tiff-pages", title: "Split & Combine TIFF", shortTitle: "TIFF Pages", blurb: "Break a multi-page TIFF into images, or bind a batch of images into one.", category: .images, pane: { TIFFView(utility: $0) }),
        Utility(id: "image-blur-faces", title: "Blur Faces", shortTitle: "Blur Faces", blurb: "Detect faces on-device and blur them — photos never leave this Mac.", category: .images, pane: { ImageBlurFacesView(utility: $0) }),
        Utility(id: "image-remove-bg", title: "Remove Background", shortTitle: "Cutout", blurb: "Lift the subject out of a photo into a transparent PNG — on-device with Vision.", category: .images, pane: { ImageRemoveBackgroundView(utility: $0) }),
    ]
}
