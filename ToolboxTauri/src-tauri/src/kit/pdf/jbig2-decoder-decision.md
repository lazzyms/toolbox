# JBIG2 decoder for Extract from PDF

- Status: accepted for #630, from the spike in #623 (parent spec #426)
- Date: 2026-11-05

## Decision

Use **hayro-jbig2 0.4.0** (`default-features = false, features = ["std"]`) to
decode `JBIG2Decode` images in process.

The spike adds it as a dev-dependency only. #630 moves it to `[dependencies]`
and regenerates `THIRD-PARTY-NOTICES` with `node scripts/third-party-notices.mjs`.
The Apache-2.0 and MIT texts are already in `scripts/third-party/licenses`.

## Why

- **Licence.** hayro-jbig2 and its only dependency, hayro-ccitt 0.5.0, are both
  `Apache-2.0 OR MIT`. Toolbox is MIT, so either licence works.
- **Pure Rust.** The crate has no build script, no C code and no `-sys`
  dependency, and it sets `#![forbid(unsafe_code)]`. No helper or pack is
  bundled.
- **Coverage.** It supports every segment type a PDF uses: generic regions,
  text regions, symbol dictionaries, refinement, pattern and halftone regions,
  and Huffman tables. `Page::new_embedded(stream, Option<&Globals>, settings)`
  takes the PDF stream and its `/JBIG2Globals` stream directly.
- **Exact fixture match.** It decodes all three fixtures pixel for pixel. It
  also decodes them after they are written to a PDF and read back with lopdf.
- **No end-of-page needed.** It accepts streams that have no end-of-page
  segment, as the fixtures do.
- **Maturity.** It is part of the hayro PDF renderer, the most-used JBIG2
  crate on crates.io, and is still maintained (0.4.0 published 2026-11-02).
- **Bonus.** hayro-ccitt can also cover #426's `CCITTFaxDecode` images.

Turn off default features:

- `image` adds an `image::ImageDecoder` hook that Toolbox does not need.
- `simd` pulls in `fearless_simd`, which uses unsafe code and only speeds up
  halftone regions. It made no measurable size difference.

## Release size

Each crate was measured in a minimal binary that decodes a stream read from a
file at run time, so the decoder code cannot be optimised out. The binary used
Toolbox's release profile: `opt-level = "s"`, `lto`, `codegen-units = 1`,
`strip`. Target: aarch64-apple-darwin, rustc 1.98.0.

| Build | Stripped binary | Added | `__text` |
|---|---:|---:|---:|
| Baseline, no decoder | 302,816 B | - | 195,716 B |
| hayro-jbig2, `std` only (chosen) | 534,304 B | +231,488 B (~226 KiB) | 371,684 B |
| hayro-jbig2, `std` + `simd` | 534,304 B | +231,488 B | 368,948 B |
| jbig2-decoder 0.1.2 | 468,400 B | +165,584 B | 334,004 B |
| justbig2 0.2.0 | 336,000 B | +33,184 B | 220,416 B |

Mach-O segments on arm64 are padded to 16 KiB. So file sizes move in 16 KiB
steps, and `__text` is the finer measure.

The full Tauri app was not rebuilt with and without the decoder. Expect about
230 KiB on macOS arm64. The Windows size was not measured.

## Platforms

| Target | Result |
|---|---|
| aarch64-apple-darwin | Builds, links and passes `cargo test` locally. |
| x86_64-pc-windows-msvc | `cargo check --release` of the minimal crate passes, with and without `simd`. Linking and running are not verified locally (no MSVC linker). |
| aarch64-pc-windows-msvc | Same as x86_64-pc-windows-msvc. |

CI coverage:

- `.github/workflows/tauri-ci.yml` runs `cargo test -- --test-threads=1` on
  `macos-26` and `windows-latest` (x86_64) only.
- `.github/workflows/tauri-release.yml` runs it on all three targets, including
  `windows-11-arm`, but only on release pushes to `main`.

So Windows arm64 is first exercised at release time unless #630 adds it to
`tauri-ci.yml`. The crate's MSRV of 1.92 is below the stable toolchain the
workflows install.

## Candidates

| Crate | Licence | Fixtures | Verdict |
|---|---|---|---|
| hayro-jbig2 0.4.0 | Apache-2.0 OR MIT | All 3 exact, with or without end-of-page | **Chosen** |
| jbig2-decoder 0.1.2 (microsoft/jbig2_rs) | MIT | All 3 exact only after an end-of-page segment is added | Rejected (see below) |
| justbig2 0.2.0 | MIT OR Apache-2.0 | Generic region only (see below) | Rejected (see below) |
| pdfluent-jbig2 0.3.0 | MIT OR Apache-2.0 | Not run | Rejected (see below) |
| aphrody-hayro-jbig2 0.3.0 | - | Not run | Rejected: an unexplained fork of hayro-jbig2 0.3, published 2026-11-03 |
| jbig2dec, jbig2dec-sys, nipdf-jbig2dec | GPL-3.0 crates over AGPL-3.0 C | Not run | Rejected: copyleft and a C build |
| pdfium-render, mupdf | - | Not run | Rejected: need a bundled pdfium library, or AGPL C |
| jbig2enc, jbig2enc-rust | - | Not run | Not decoders (jbig2enc also needs leptonica C) |

Why the candidates were rejected:

- **jbig2-decoder.** It was first published on 2026-11-02, and its own README
  calls it not yet suitable for untrusted production documents. It returns
  `MissingEndOfPage` for PDF streams that omit the end-of-page segment, as the
  fixtures do. The fixtures had to add that segment before it would decode
  them.
- **justbig2.**
  - Its source says it is ported from jbig2dec, which is AGPL-3.0, so its
    licence provenance is in doubt.
  - It returns no pages unless the stream has an end-of-page segment.
  - With that segment added, it still returns blank pages for both
    text-region fixtures, with no error.
- **pdfluent-jbig2.** It is a fork of an older hayro-jbig2, and its README
  points back to upstream.

## Fixtures

The fixtures are generated in code in `jbig2_fixtures.rs` (test-only, for reuse
by #630). A small MQ arithmetic encoder writes the region data, so every
expected pixel is known.

- **`generic_region()`** is a 23x9 immediate generic region: template 0,
  nominal adaptive pixels, TPGDON off.
- **`text_region_with_symbol_dictionary()`** is a 24x8 text region using
  TOPLEFT and two strips. It refers to a symbol dictionary in the same stream.
  The dictionary holds three symbols in two height classes, all exported.
- **`text_region_with_embedded_globals()`** has the same pixels, but the
  symbol dictionary is in a separate global stream with page association 0.
- **`pdf_with_images()`** puts each fixture in a PDF as a `JBIG2Decode` image,
  with `/DecodeParms /JBIG2Globals` where the fixture has globals.

hayro-jbig2 and jbig2-decoder decode all three to the same pixels. jbig2-decoder
needs an end-of-page segment first. This is an independent check that the
fixtures follow T.88 and do not just match one decoder.

The fixtures do not cover:

- MMR or Huffman coding;
- refinement;
- pattern and halftone regions;
- generic templates 1 to 3;
- TPGDON;
- striped pages.

## Notes for #630

- **Invert the pixels.** JBIG2 uses 1 for black. A `JBIG2Decode` image in
  `DeviceGray` reads 0 as black, so invert before writing grey pixels, or
  honour `/Decode`.
- **Rows are padded.** `push_pixel_chunk` can pad a row to a multiple of 8
  pixels, so truncate each row to `page.width()`.
- **Choose strict or lenient.** Decide whether to use `DecodeSettings { strict }`.
  The spike uses `strict: true`, and lenient may suit real scans better.
- **Set limits before decoding.** Check `page.width()` and `page.height()`
  against the existing image limits before `decode`.
- **Replace the spike test.** `jbig2_spike.rs` is throwaway; replace it with
  coverage through `extract_images`.

## Outcome in #630

- `hayro-jbig2` is a normal dependency. `embedded_image.rs` decodes
  `JBIG2Decode` (with `/JBIG2Globals`, whose own filters are undone first) into
  1-bit rows with 0 for black, so the existing colour space, `/Decode` and
  `/ImageMask` handling applies.
- Decoding is **strict**. Lenient mode skips a region it cannot decode and
  returns the rest of the page, which would save a blank or partial PNG with no
  report. A strict failure is reported as damaged for that image alone;
  `DecodeError::Unsupported` (colour extension, required extensions) is
  reported as "JBIG2 isn't supported in this version".
- `jbig2_spike.rs` was removed. `remaining.rs` covers the fixtures through
  `extract_images`.
