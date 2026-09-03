# Tauri image-format fidelity decision research

## Scope

This note resolves the image-format question for the Tauri parity map. The
legacy implementation is the behavioral oracle; the Rust implementation must
match its visible outcomes and the acceptance contract from
[Define the parity acceptance contract for all migrated tools](https://github.com/lazzyms/toolbox/issues/117).

## Findings

- The Rust `image` crate provides GIF and TIFF decoding/encoding and WebP
  decoding, but documents WebP encoding as lossless-only. Lossy WebP therefore
  needs the existing `webpx` adapter (or another libwebp-backed adapter).
  [image codec support](https://docs.rs/image/latest/image/codecs/index.html)
- WebP's container can carry alpha, ICC profiles, animation, EXIF, and XMP.
  Decoding a single `DynamicImage` and writing it back cannot preserve those
  container-level features by itself.
  [WebP container specification](https://developers.google.com/speed/webp/docs/riff_container)
- The `image` crate exposes orientation extraction, pixel baking, and EXIF
  orientation reset. Any operation that bakes orientation must clear the tag,
  otherwise compliant readers apply the transform twice.
  [image orientation API](https://docs.rs/image/latest/image/metadata/enum.Orientation.html)
- HEIC is outside the `image` crate's built-in codec list. The existing
  `heif-rs` dependency is therefore the required HEIF boundary, and its
  availability must be tested per packaged target rather than inferred from a
  file extension.
- The current Tauri paths lose or mis-handle fidelity: generic transforms use
  `image::open`, metadata stripping re-encodes without stripping metadata,
  TIFF processing handles one decoded image rather than an explicit page
  sequence, GIF creation fixes every delay to 100 ms, and icon generation only
  emits generic PNG sizes. These differ from the legacy app's metadata,
  animation, multi-page, and preset behavior.

## Decision

1. **Pass-through.** Only a deliberate no-op may copy source bytes unchanged.
   The copy must retain the original extension and be reported as a pass-through
   result. No transformed output may be called lossless merely because its
   decoded pixels match.
2. **Single-frame transforms.** Decode to a frame model, apply EXIF
   orientation exactly once, preserve alpha for PNG/WebP/HEIC-capable outputs,
   and clear the baked orientation tag. JPEG output is explicitly opaque and
   may flatten alpha according to the documented conversion policy.
3. **Lossless promises.** PNG, lossless WebP, and TIFF operations that promise
   lossless behavior must assert pixel equality plus dimensions, alpha, color
   profile, and required metadata invariants. Byte identity is required only
   for operations that explicitly promise preservation; otherwise canonical
   re-encoding is allowed but must not be mislabeled as byte-preserving.
4. **Lossy promises.** JPEG, lossy WebP, and HEIC use the requested quality and
   are validated by dimensions, alpha policy, orientation, required metadata
   policy, and bounded perceptual difference against the legacy golden output.
   The existing no-inflation guard remains mandatory for compression.
5. **Animation.** Animated GIF is a frame sequence, not a still image. Create,
   extract, convert, and compress must either preserve frame count, order,
   canvas, disposal, loop count, and frame delays, or fail clearly. A still-only
   operation must not silently flatten animation. Animated WebP is accepted
   only through a sequence-capable adapter; otherwise it is an explicit
   unsupported-input result.
6. **TIFF.** Split/combine must use an explicit multi-page reader/writer and
   preserve page count, page order, dimensions, color type, alpha, and relevant
   TIFF tags. A one-frame decode is not sufficient for the multi-page tool.
7. **Metadata.** Metadata removal is a container operation: remove EXIF/GPS,
   XMP, ICC, and format-specific metadata required by the selected legacy mode,
   then verify absence. If a codec cannot surgically remove metadata, rewrite
   pixels through a metadata-free encoder and state the resulting metadata
   policy; never claim "without recompressing" when recompression occurred.
8. **App icons.** Port the legacy presets as named plans (macOS iconset,
   favicon, iOS, Android, and custom sizes), not as one generic size loop.
   Validate exact output names, dimensions, alpha, and required container
   artifacts. Unsupported platform artifacts must be feature-flagged only
   after packaging analysis, not silently omitted.
9. **Unsupported inputs.** Surface an actionable error for missing codec,
   encrypted/corrupt data, unsupported bit depth, unsupported animation, or
   unpreservable metadata. Never fall back from a multi-frame or multi-page
   input to frame/page one without telling the user.

## Implementation consequences

The next implementation should introduce a shared frame/container abstraction
with separate single-image and sequence paths. It should add golden fixtures
for oriented JPEG, transparent PNG, lossless/lossy WebP, HEIC, animated GIF,
multi-page TIFF, metadata-bearing files, and every icon preset. The current
Tauri commands should not be considered parity-complete until those fixtures
exercise both adapter-present and adapter-absent cases.

