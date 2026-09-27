# Offline vision engines

On `main`, OCR PDF, Blur Faces, and Remove Background are registered as
`implemented`; the release build stages their offline adapters and resources.
The latest published application release may predate this source change, so
check its version before claiming that a downloadable build includes these tools.

| Feature | Adapter variable | Default executable | Output |
| --- | --- | --- | --- |
| PDF OCR | `TOOLBOX_TESSERACT_PATH` | `tesseract` | `-ocr-text.txt` |
| Face blur | `TOOLBOX_FACE_BLUR_PATH` | `toolbox-face-blur` | `-blurred.png` |
| Background removal | `TOOLBOX_BACKGROUND_REMOVAL_PATH` | `toolbox-background-removal` | `-cutout.png` |

Adapters receive the input path followed by the output path, except OCR, which
receives the input path and writes text to `stdout`. The release build stages the
vision adapter, Tesseract, and their model/data assets for supported platform
targets. The application does not fall back to a network service or create an
identity copy.

The native resolver checks the bundled `vision/manifest.json` first. Development
overrides use the variables in the table, and PATH lookup is the final fallback
when no bundled manifest is present. Every bundled resource must declare a
version, architecture, license, size limit, and SHA-256 checksum. The resolver
rejects a missing, mismatched, or over-limit resource.

When a resource is available, the native validators require OCR text or a valid
PNG result. Face blur must preserve dimensions and show a changed face region.
Background removal must produce a readable PNG with a transparent subject mask.
