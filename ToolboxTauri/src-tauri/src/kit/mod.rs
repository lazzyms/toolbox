pub mod common;
pub mod contracts;
pub mod images;
pub mod pdf;
pub mod vision;
pub mod fidelity;
pub mod resources;

#[cfg(test)]
mod fidelity_tests {
    use super::fidelity::{describe, ContentShape, MetadataPolicy, OrientationPolicy};
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Frame, RgbaImage};
    use lopdf::{dictionary, Document, Object, Stream};
    use std::fs::File;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("toolbox_fidelity_{}_{}", std::process::id(), name))
    }

    #[test]
    fn describes_a_png_as_one_frame() {
        let path = temp_path("single.png");
        RgbaImage::new(2, 3).save(&path).unwrap();

        let model = describe(&path).unwrap();
        assert_eq!(model.shape, ContentShape::SingleFrame { width: 2, height: 3 });
        assert_eq!(model.orientation, OrientationPolicy::BakePixelsOnce);
        assert_eq!(model.metadata, MetadataPolicy::PreserveUnlessToolRemoves);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_tiff_without_a_page_aware_operation() {
        let path = temp_path("multi-page.tiff");
        std::fs::write(&path, b"not a tiff").unwrap();

        let model = describe(&path).unwrap();
        assert!(matches!(model.shape, ContentShape::Unsupported { .. }));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn describes_an_animated_gif_as_a_frame_sequence() {
        let path = temp_path("animated.gif");
        let mut file = File::create(&path).unwrap();
        let mut encoder = GifEncoder::new(&mut file);
        encoder.set_repeat(Repeat::Infinite).unwrap();
        encoder.encode_frame(Frame::new(RgbaImage::new(2, 3))).unwrap();
        encoder.encode_frame(Frame::new(RgbaImage::new(2, 3))).unwrap();
        drop(encoder);
        drop(file);

        let model = describe(&path).unwrap();
        assert_eq!(model.shape, ContentShape::FrameSequence { count: 2, width: 2, height: 3 });
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn describes_pdf_pages_without_flattening_them() {
        let path = temp_path("pages.pdf");
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let content_id = document.add_object(Stream::new(dictionary! {}, Vec::new()));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "MediaBox" => vec![0.into(), 0.into(), 10.into(), 20.into()],
            "Resources" => dictionary! {}, "Contents" => content_id,
        });
        document.objects.insert(pages_id, dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1,
        }.into());
        let catalog_id = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);
        document.save(&path).unwrap();

        let model = describe(&path).unwrap();
        assert_eq!(model.shape, ContentShape::PageSequence { count: 1 });
        let _ = std::fs::remove_file(path);
    }
}
