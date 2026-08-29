# Image Format Fidelity Research: Rust vs. Apple ImageIO

This report evaluates the capabilities of the Rust `image` crate and its ecosystem compared to Apple's `ImageIO` framework, specifically for the Tauri Toolbox project.

## Executive Summary

Rust's image processing ecosystem can match the fidelity and functionality of Apple's `ImageIO`, but it requires a combination of crates rather than a single monolithic framework. While the `image` crate provides a great general-purpose API, specialized formats like HEIC and lossy WebP require additional crates (`heif-rs` and `webpx`).

## Detailed Findings

### 1. HEIC Support (Decode & Encode)
Apple's `ImageIO` has first-class support for HEIC. The Rust `image` crate **does not natively support HEIC**.

**Rust Alternatives:**
- **`heif-rs`**: A high-level wrapper around `libheif`. It is recommended for ease of setup as it bundles prebuilt static binaries for `libheif`, `x265`, and `libde265`. It provides simple `decode` and `encode` functions that integrate directly with the `image` crate's `DynamicImage`.
- **`libheif-rs`**: A more comprehensive wrapper for those who need granular control over the HEIF container, metadata, and system-level library management.

| Capability | ImageIO | Rust Ecosystem (`heif-rs` / `libheif-rs`) | Status |
| :--- | :--- | :--- | :--- |
| Decode HEIC | ✅ | ✅ | Match |
| Encode HEIC | ✅ | ✅ | Match |

### 2. WebP Support (Decode & Encode)
`ImageIO` handles WebP transparently. The `image` crate has built-in WebP support, but it is limited for encoding.

**Rust Capabilities:**
- **Decoding**: Fully supported in the `image` crate (lossy and lossless).
- **Encoding**:
    - The `image` crate only supports **lossless** WebP encoding.
    - **`webpx`**: Safe bindings to Google's official `libwebp` library. This provides full **lossy** encoding support with quality settings (0-100) and content-aware presets (Photo, Drawing, etc.).
    - **`webp-rust`**: A pure Rust implementation providing basic lossy `VP8` encoding.

| Capability | ImageIO | Rust Ecosystem (`image` + `webpx`) | Status |
| :--- | :--- | :--- | :--- |
| Decode WebP | ✅ | ✅ | Match |
| Encode Lossless | ✅ | ✅ | Match |
| Encode Lossy | ✅ | ✅ (via `webpx`) | Match |

### 3. No-Inflation Guard
The requirement is to ensure that a "compressed" file does not end up larger than the original.

**Implementation in Rust:**
This is highly feasible. Unlike some frameworks that write directly to a file stream, the Rust encoders for WebP (`webpx`) and HEIC (`heif-rs`) typically return the encoded result as a `Vec<u8>`. 

**Proposed Logic:**
1. Read original file size (`S_orig`).
2. Encode image to a memory buffer (`Vec<u8>`).
3. Compare buffer length (`S_new`) with `S_orig`.
4. If `S_new > S_orig`, discard the buffer and copy the original bytes to the destination.

| Capability | ImageIO | Rust Ecosystem | Status |
| :--- | :--- | :--- | :--- |
| Byte-level guard | ✅ | ✅ | Match |

### 4. EXIF Orientation & Pixel Baking
`ImageIO` handles orientation via metadata flags. To match the "Toolbox" behavior of baking orientation into pixels:

**Rust Implementation:**
The `image` crate (v0.25.8+) provides a streamlined API for this:
1. **Extraction**: Use `decoder.orientation()` to get the `Orientation` enum from the image metadata.
2. **Baking**: Call `img.apply_orientation(orientation)`, which physically rearranges the pixel buffer (rotates/flips).
3. **Metadata Cleanup**: Use `Orientation::remove_from_exif_chunk` to reset the orientation tag to `NoTransforms` (Value 1) in the output EXIF data, preventing "double rotation" in image viewers.

| Capability | ImageIO | Rust Ecosystem (`image`) | Status |
| :--- | :--- | :--- | :--- |
| Read Orientation | ✅ | ✅ | Match |
| Physical Bake | ✅ | ✅ | Match |
| Metadata Reset | ✅ | ✅ | Match |

## Conclusion and Recommended Stack

To achieve parity with the existing macOS Toolbox implementation, the following Rust stack is recommended:

- **`image`**: Core image manipulation and decoding.
- **`heif-rs`**: For HEIC decode/encode (easiest setup).
- **`webpx`**: For professional-grade lossy WebP encoding.
- **`exif`**: For advanced metadata manipulation if `image` crate's built-in helpers are insufficient.

## Sources
- [image::codecs - Rust](https://docs.rs/image/latest/image/codecs/index.html)
- [heif-rs - crates.io](https://crates.io/crates/heif-rs)
- [libheif-rs - docs.rs](https://docs.rs/libheif-rs/latest/libheif_rs/index.html)
- [webpx - docs.rs](https://docs.rs/webpx/latest/webpx/)
- [webp-rust - crates.io](https://crates.io/crates/webp-rust)
- [Orientation in image::metadata - Rust](https://docs.rs/image/latest/image/metadata/enum.Orientation.html)
- [Resizing images in Rust with EXIF orientation - alexwlchan](https://alexwlchan.net/2025/create-thumbnail-is-exif-aware/)
