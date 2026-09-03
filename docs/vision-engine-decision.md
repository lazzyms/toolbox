# Portable offline vision engine decision

## Scope

This note resolves the engine-selection question for OCR, face blur, and
background removal in the Tauri parity map. The legacy behavior remains the
user-visible oracle; model outputs are therefore validated against fixtures
and are never treated as byte-deterministic.

## Existing behavior

The Swift app uses Vision for accurate English OCR, face rectangles, and
`VNGenerateForegroundInstanceMaskRequest`. The Tauri shell currently invokes
external adapters and correctly refuses to pretend that an unavailable model
worked. The adapter boundary should remain, but the release must supply a
known adapter and pinned assets on every supported target.

## Decision

### OCR: Tesseract 5

Use a bundled Tesseract 5 adapter with the `eng` traineddata asset for the
initial parity surface. Tesseract is Apache 2.0, documents both macOS and
Windows installation paths, and separates the engine from language data. Keep
the adapter protocol as `input path -> stdout`, but add explicit language,
page-render DPI, timeout, and memory-limit arguments rather than relying on
machine-global defaults.

This is a behavioral substitute, not an identical Vision model. OCR fixtures
must compare normalized line text and ordering, with a reviewable mismatch
report. Additional language packs are opt-in assets with their own checksums
and notices; they are not downloaded at runtime.

### Face blur: YuNet + deterministic renderer

Use OpenCV Zoo's YuNet face-detection ONNX model, executed by the bundled
portable adapter. YuNet is lightweight and its model directory carries an MIT
license. The detector emits boxes; the existing deterministic blur pipeline
owns padding, radius, compositing, output format, and the "no faces means no
output" rule.

Pin model version, input size, confidence/NMS thresholds, and coordinate
conversion. Store model checksums in the release manifest. Fixture gates must
check detection recall on representative frontal, profile, occluded, small,
and multiple-face images; the UI must report faces found and continue to warn
that detection is not guaranteed.

### Background removal: U²-Netp first, U²-Net fallback

Use the Apache-2.0 U²-Net project models through the same portable inference
adapter. Ship U²-Netp as the default resource-constrained model and permit the
larger U²-Net model only where the memory budget allows it. U²-Net is salient
object segmentation, so its contract is "best subject cutout", not an exact
replica of Vision's instance-mask semantics. The output remains an RGBA PNG
with soft alpha edges, upright pixels, and no silent frame flattening.

Do not adopt a model with a non-commercial or unclear weight license. Do not
download weights or call a hosted API. If the model or runtime is missing,
surface the feature as unavailable and leave the source untouched.

### Runtime and bundle

Use one version-pinned CPU ONNX adapter for YuNet and U²-Net, with separate
model files and a small JSON manifest containing engine version, model
checksum, license, expected input shape, and maximum memory estimate. The
adapter must run on arm64 and x86_64 macOS and x86_64 Windows for the supported
release targets. Hardware acceleration is optional and must not alter the
CPU-reference acceptance result.

Bundle layout is private application resources, never PATH discovery:

```text
resources/
  vision/
    manifest.json
    ocr/tesseract(.exe)/tessdata/eng.traineddata
    face/yunet.onnx
    segmentation/u2netp.onnx
```

The environment-variable overrides remain developer/test escape hatches only.
Production discovery first resolves the signed, bundled adapter relative to
the application resources and validates its manifest before execution.

## Resource and update policy

- Process one input at a time per adapter, with bounded batch concurrency at
  the application layer.
- Enforce input pixel/page limits, a per-file timeout, and a memory ceiling;
  kill the child process on breach and report a typed failure.
- Ship immutable model assets with the application. Update them only through a
  signed application release, never an opaque background download.
- Record adapter/model versions in verification output so model changes create
  an intentional fixture-baseline review.
- Run adapter-present and adapter-absent tests in CI on both OS families;
  absent adapters must hide/disable the feature or show the documented
  unavailable state without writing an output.

## Primary sources

- [Tesseract installation and license documentation](https://tesseract-ocr.github.io/tessdoc/Installation.html)
- [OpenCV Zoo YuNet model repository](https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet)
- [YuNet model license](https://github.com/opencv/opencv_zoo/blob/main/models/face_detection_yunet/LICENSE)
- [U²-Net repository](https://github.com/xuebinqin/U-2-Net)
- [U²-Net Apache 2.0 license](https://github.com/xuebinqin/U-2-Net/blob/master/LICENSE)
- [ONNX Runtime installation documentation](https://onnxruntime.ai/docs/install/)

