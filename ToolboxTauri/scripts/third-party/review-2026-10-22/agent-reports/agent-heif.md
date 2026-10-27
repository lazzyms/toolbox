# HEIF stack license verification (libheif / libde265 / x265 / libwebp)

Date: 2026-10-22. Read-only check. Nothing was edited or downloaded to disk. Local evidence comes from
ToolboxTauri/src-tauri/target/debug/build/heif-rs-26a56cb6bf72affb/out/binaries-heif-26.7.0 (macOS static archive, the same contents in all 4 cached copies).

## 1. vegidio/binaries-heif @ 26.7.0

- Tag object 6d09bf70, commit fc5579cdaa4d8d41ecddf819dbb4e6d4662eb1dd, tagged 2026-07-03T09:21:07Z.
  The CI run that built the release: https://github.com/vegidio/binaries-heif/actions/runs/28651184420 (success, finished 09:45Z).
  Assets uploaded 09:45:20-23Z and never replaced. Asset digests: static_osx_arm64 sha256:92be83c8..., static_osx_x64 sha256:b9c47432..., static_windows_x64 sha256:6021c064..., static_windows_arm64 sha256:d529bdc1....
- Build scripts: https://github.com/vegidio/binaries-heif/blob/26.7.0/.github/workflows/build.yml, .github/actions/{macos,windows}/action.yml.
  They run `git clone https://github.com/microsoft/vcpkg` with **no pinned commit**, then `vcpkg install "libheif[hevc]:<triplet>"`. The triplets are x64-osx / arm64-osx (static), and x64-windows-static-md / arm64-windows-static-md.
  Then `strip -S` (macOS) or `llvm-objcopy --strip-debug` (Windows).
- The vcpkg HEAD at clone time was most likely https://github.com/microsoft/vcpkg/commit/38a77589121bd7ec6487927b79c191a75d4ebdd6, the latest master commit before 09:25Z (committed 01:50Z). This is an inference: the repo records no vcpkg commit. Ports at that commit:
  - libheif **1.23.1** (tag v1.23.1, SHA512 ebb0bf74...). Patches: cxx-linkage-pkgconfig.diff, find-modules.diff, gdk-pixbuf.patch, symbol-exports.diff, plus cmake-project-include.cmake. Only the default feature `hevc` (WITH_X265) is on. WITH_DAV1D=OFF, WITH_EXAMPLES=OFF, WITH_LIBSHARPYUV=OFF, OpenH264 OFF, Brotli/PNG/TIFF lock OFF. aom, jpeg, openjpeg, zlib (iso23001-17), x264 and gdk-pixbuf are all off. https://github.com/microsoft/vcpkg/tree/38a77589121bd7ec6487927b79c191a75d4ebdd6/ports/libheif
  - libde265 **1.1.1** (v1.1.1). Patches: fix-linkage.patch, fix-api-visibility.patch. ENABLE_SDL=OFF. https://github.com/microsoft/vcpkg/tree/38a77589121bd7ec6487927b79c191a75d4ebdd6/ports/libde265
  - x265 **4.2** (Bitbucket multicoreware/x265_git ref 4.2, SHA512 53fe7b3a...). Patches: disable-install-pdb, version, linkage, pkgconfig, pthread, compiler-target, neon, advapi32. ENABLE_PIC=ON, ENABLE_LIBNUMA=OFF, ENABLE_ASSEMBLY=OFF on Windows (NASM on macOS x64). https://github.com/microsoft/vcpkg/tree/38a77589121bd7ec6487927b79c191a75d4ebdd6/ports/x265
- The local binaries confirm this:
  - The headers report LIBHEIF_VERSION "1.23.1" and LIBDE265_VERSION "1.1.1".
  - libx265.a contains the string "4.2-vcpkg" and X265_BUILD 216.
  - Upstream X265_BUILD is 215 for 4.1 and 216 for 4.2 (bitbucket.org/multicoreware/x265_git/raw/{4.1,4.2}/source/CMakeLists.txt). **So X265_BUILD 216 is x265 4.2, not 4.1.**
  - The lib/ folder holds only libheif.a, libde265.a and libx265.a.
  - libheif.a has plugins libde265 (decoder), x265 (encoder) and the built-in mask encoder.
  - No undefined zlib, jpeg, brotli, openjpeg, aom, dav1d or webp symbols.
- The release notes claim "libheif v1.23.0, libde265 v1.1.0, x265 v4.1". **That is wrong, or stale from an earlier release.** The notes body is static text in the workflow ("New release") and was edited by hand afterwards.
- The repo has no LICENSE (GitHub license: null). Its README has no license text and no list of sources. heif-rs 26.7.0 is Apache-2.0. Its build.rs downloads the asset by URL and does **not** check a checksum.
- Is the corresponding source exactly identified? **No, not from a complete tagged public record.** The scripts are tagged, but the vcpkg revision is not pinned or logged in the repo. The CI logs show it, but logs expire.
  The exact source can be rebuilt from upstream tags plus vcpkg port patches at commit 38a7758, but that commit is an inference. Toolbox would have to name it (or vendor the patches) itself.

## 2. Upstream licenses vs native-components.json

- libheif v1.23.1:
  - COPYING says the library is LGPL and that the "sample applications and the Go and C++ wrappers" are MIT.
  - Source headers say "LGPL version 3 ... or (at your option) any later version", so LGPL-3.0-or-later. vcpkg labels it LGPL-3.0-only, which is less accurate.
  - Copyright lines: README has "(c) 2017-2020 Struktur AG" and "(c) 2017-2026 Dirk Farin". heif.h has "Copyright (c) 2017-2025 Dirk Farin".
  - The shipped heif_cxx.h is MIT (header only; not used by heif-rs).
- libde265 v1.1.1: library LGPL-3.0-or-later (de265.h header), samples MIT. Copyright "(c) 2013-2014 Struktur AG", "(c) 2013-2026 Dirk Farin".
- x265 4.2: GPL-2.0-or-later (x265_config.h, COPYING GPLv2). "This program is also available under a commercial proprietary license ... license @ x265.com". Copyright "(C) 2013-2020 MulticoreWare, Inc" in headers.
- libwebp (libwebp-sys 0.14.4 vendor = libwebp **1.6.0**, DEC_*_VERSION 1.6.0): BSD-3-Clause, "Copyright (c) 2010, Google Inc. All rights reserved." It includes a separate PATENTS file ("Additional IP Rights Grant (Patents)"). BSD-3 requires the COPYING text. PATENTS is a grant rather than a condition, but it is customarily shipped with it.

Mismatches in ToolboxTauri/scripts/third-party/native-components.json:
1. libheif version 1.23.0 should be 1.23.1. Source URL should be https://github.com/strukturag/libheif/archive/refs/tags/v1.23.1.tar.gz.
2. libheif copyright: "struktur AG and the libheif contributors" leaves out Dirk Farin. Suggest "Copyright (c) 2017-2020 Struktur AG; Copyright (c) 2017-2026 Dirk Farin".
3. libde265 version 1.1.0 should be 1.1.1. Source URL should be .../libde265/archive/refs/tags/v1.1.1.tar.gz. Copyright should add Dirk Farin (2013-2026).
4. x265 version 4.1 should be 4.2. Source x265_4.1.tar.gz should be the 4.2 source (Bitbucket tag 4.2, https://bitbucket.org/multicoreware/x265_git/src/4.2/, or the videolan release tarball). Copyright "MulticoreWare, Inc" is OK, with years 2013-2020 per headers.
5. No entry mentions the vcpkg patches or the vcpkg commit, which corresponding source needs. Also the binaries-heif build scripts (tag 26.7.0).
6. libwebp: no version given (1.6.0). Source should point at tag v1.6.0. The PATENTS grant is not referenced.
7. The license IDs for libheif and libde265 (LGPL-3.0-or-later) and x265 (GPL-2.0-or-later) are correct.

## 3. HEVC patents (facts only)

- x265 docs (doc/reST/introduction.rst at 4.2) say the GPL/commercial license covers copyright, "but do not cover any patents". It says the distributor is "responsible for ... licensing all applicable patent rights". It points to the MPEG LA HEVC licensing program.
- libheif and libde265 README/COPYING at these tags contain no patent statement.
- HEVC is covered by patent pools (MPEG LA/Via LA, Access Advance). No legal conclusion is drawn here.
