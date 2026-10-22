# Third-party provenance (review of 2026-10-22)

This list records where each fact in `native-components.json` comes from, and
what is still missing. Evidence files are in `review-2026-10-22/`. It records
facts and open questions. It is not legal advice.

**Pinned** means the CI input is fixed by a version or a checksum.
**Observed** means the value was read from shipped binaries or from a package
that CI installs unpinned, so a later build can differ.

## Inputs by build

| Input | Used by | Pinned? | Evidence |
|---|---|---|---|
| binaries-heif 26.7.0 (libheif, libde265, x265) | app binary, all platforms | release tag only, no hash check in heif-rs | `agent-reports/agent-heif.md` |
| libwebp-sys (Cargo.lock) | app binary | yes | `Cargo.lock` |
| qpdf 12.4.2 MSVC | GitHub Windows installers | yes (`scripts/qpdf-pin.json`) | `scripts/qpdf-pin.json` |
| Chocolatey `qpdf` | Microsoft Store build | **no** (`microsoft-store.yml`: `choco install qpdf tesseract -y`) | `agent-reports/agent-poppler.md` |
| Chocolatey `tesseract` 5.5.3.20260724 | Windows OCR helper | **no** | `agent-reports/agent-tesseract.md` |
| poppler-windows 26.02.0-0 zip | Windows PDF helper | yes (sha256) | `agent-reports/agent-poppler.md` |
| Homebrew poppler, qpdf, tesseract and their dylibs | macOS | **no** | `agent-reports/agent-mac.md`, `maccov-closure-files.txt` |

## Corrections made, with evidence

| Component | Change | Evidence |
|---|---|---|
| libheif, libde265, x265 | 1.23.1 / 1.1.1 / 4.2, exact tarballs; libheif and libde265 holders from upstream COPYING | version headers and `libx265.a` strings (`agent-heif.md`) |
| libwebp | 1.6.0 tarball; PATENTS reproduced | `upstream/libwebp-PATENTS`, `notices/libwebp-1.6.0-PATENTS.txt` |
| qpdf | holders Jay Berkenbilt / Jay Berkenbilt and Manfred Holger; NOTICE.md reproduced; Store build's static OpenSSL, zlib and libjpeg-turbo noted | qpdf 12.4.2 NOTICE.md; DLL strings (`agent-poppler.md`) |
| Poppler | `GPL-2.0-only OR GPL-3.0-only`; conda-forge build h4b9d284_3 and feedstock commit b46a81fb3d74 | `agent-poppler.md` |
| Tesseract | 5.5.3; official installer cross-built with Debian mingw (not UB Mannheim); Hewlett-Packard line | DLL strings; `src/ccstruct/blobbox.cpp` header (`agent-tesseract.md`) |
| Pango | `LGPL-2.0-or-later`; Red Hat Software | Pango COPYING and headers |
| HarfBuzz | `MIT-Modern-Variant`; COPYING reproduced | `upstream/harfbuzz-COPYING` |
| Fontconfig | full COPYING embedded; compound license id | `upstream/fontconfig-COPYING`, `upstream/brew-fontconfig.json` |
| Zstandard | `BSD-3-Clause OR GPL-2.0-only` | zstd LICENSE and COPYING |
| libarchive | `BSD-2-Clause AND BSD-3-Clause`; COPYING reproduced for the public-domain and BLAKE2 parts | `upstream/libarchive-COPYING` |
| LERC | NOTICE reproduced; Esri 2015-2026 | `upstream/lerc-NOTICE-4.1.0` |
| pixman, OpenJPEG, libssh2, PCRE2, winpthreads | named holders; upstream COPYING/LICENSE/LICENCE reproduced | files under `notices/` |
| FriBidi, libpng, Leptonica, Little CMS, zlib, libdeflate, OpenSSL, libpsl, libffi | holders and years as in upstream license files | `upstream/` |
| libthai and libdatrie | libdatrie source added | tlwg GitHub |
| GCC runtime | OCR copies from Debian gcc-mingw-w64 GCC 14; Store qpdf copies from MSYS2 GCC 16.2.0 | DLL strings (`agent-tesseract.md`, `agent-poppler.md`) |
| OpenSSL | renamed "OpenSSL (libssl, libcrypto)"; dylib glob narrowed to `libssl.*.dylib` so NSS's `libssl3.dylib` is no longer credited to OpenSSL | test `each macOS TLS library is credited to the project that ships it` |
| GPGME, GpgME++, libassuan, libgpg-error, NSS, NSPR | new macOS-only entries | macOS CI run 39657757927 job 119422452159 listed these 10 dylibs as unlisted (`validation/macos-ci-run-39657757927.txt`); `upstream/*-AUTHORS`, `upstream/brew-*.json` |
| DejaVu | `LicenseRef-Arev` (Arev is not an SPDX id); public-domain line | font LICENSE.txt |
| Satisfy | Font Diner, Inc. copyright and trademark | font name table and license |
| Pacifico | version 3.001 (also in `fonts/manifest.json`) | font name table |
| Per-library Windows versions | added to each `note`; values marked "inferred" have no version string in the binary and come from MSYS2 package history | `agent-tesseract.md`, `agent-poppler.md` |

## Not proved: missing evidence and next step

| Gap | Missing evidence | Next step |
|---|---|---|
| vcpkg commit for binaries-heif 26.7.0 | binaries-heif records no vcpkg commit; 38a77589121b is only inferred | Ask the binaries-heif maintainer for the commit, or build the HEIF archive in this repo with a pinned vcpkg baseline and record hashes |
| binaries-heif scripts licence | that repo has no LICENSE | Ask the maintainer to add one before mirroring its scripts as corresponding source |
| Poppler conda solve | poppler-windows build log expired (HTTP 410); no solved package list | Record `conda list --explicit` when building a replacement zip, or mirror the feedstock commit and the package tarballs |
| Tesseract MSYS2 package versions | no manifest in the installer; pkgrel values marked "inferred" are from MSYS2 history | Ask upstream to publish the `pacman -Q` list, or rebuild the installer and record it |
| Debian gcc-mingw-w64 revision | the DLLs show GCC 14 only | Record the package revision from the installer workflow log, if available |
| Homebrew versions | no `tap_git_head` or bottle hashes recorded; formula versions already differ from the review machine (for example poppler 26.10.0 vs 26.04.0) | Record `brew info --json=v2 --installed` in the macOS job and keep it with each release |
| Chocolatey qpdf and tesseract | not pinned | Add `--version` to `choco install` in `microsoft-store.yml` |
| Corresponding source for GPL parts (x265, Poppler, JBIG-KIT) | nothing hosts the exact source or makes a written offer | Publish a source bundle per release and link it from the notices |
| LGPL relinking (libheif and libde265 statically linked; LGPL DLLs inside a signed MSIX) | no confirmation that users can relink or replace them | Counsel review |
| HEVC patents (x265) | no patent position | Counsel review, or check whether HEVC encoding is needed |
| Store policy on GPL binaries | not confirmed | Check Microsoft Store policy with counsel |
| u2netp.onnx origin | size 4574861 matches the rembg v0.0.0 asset, but that asset has no published digest; our sha256 starts 309c8469 and ends 4ddd8 | Ask rembg for the digest, or download the asset and compare hashes, before naming rembg (MIT, Daniel Gatis) in the notice |
| GpgME++ licence | upstream AUTHORS says LGPL-2.0-or-later; Homebrew says LGPL-2.1-or-later | The notice follows upstream; confirm with the gpgmepp maintainers |
| x265 "whole binary is GPL-3.0-or-later" note | a licensing-strategy question, left unchanged | Counsel review |
| Windows MSIX contents | no `toolbox-msix` artifact was built | Rerun the Store workflow, then run `--binaries` on the unpacked MSIX |

## Validation (saved in `review-2026-10-22/validation/`)

- `notices-generate-check.txt`: notices regenerated; `--check` passes.
- `node-tests.txt`: notices and asset-config tests, 9 of 9 pass.
- `macos-binaries-coverage.txt`: `--binaries` passes on the macOS dylib closure collected during the review.
- `check-release.txt`: `npm run check:release` passes.
- `cargo-test.txt`: 282 passed.
- `macos-ci-run-39657757927.txt`: the failing macOS CI log (read-only), which lists the dylibs now covered.
