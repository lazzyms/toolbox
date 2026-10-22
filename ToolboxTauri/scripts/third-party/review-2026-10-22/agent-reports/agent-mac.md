# License verification: qpdf (Windows), macOS Homebrew helpers, fonts and models

Data file: ToolboxTauri/scripts/third-party/native-components.json. Checked 2026-10-22. Facts and obligations only.

## 1. qpdf on Windows

- Two install paths exist:
  - tauri-release.yml (GitHub installers) runs scripts/bundle-qpdf.ps1, which uses the pinned, SHA-256-checked `qpdf-12.4.2-msvc64.zip` / `msvc-arm64.zip` (ToolboxTauri/qpdf-pin.json).
  - tauri-ci.yml and microsoft-store.yml run an unpinned `choco install qpdf`.
- Chocolatey `qpdf` is currently 12.4.2 (updated 29 Sep 2026). Its install script does not download anything. It unzips `qpdf-12.4.2-mingw64_x64.zip` from its own tools folder, which its verification notes say is upstream `qpdf-12.4.2-mingw64.zip` (https://community.chocolatey.org/packages/qpdf).
- The MSIX 1.0.17 qpdf is the **MinGW** build, not MSVC:
  - qpdf.exe has sha256 7a155e0f… and qpdf30.dll has sha256 fc358bbc….
  - Both contain the string `GCC: (Rev4, Built by MSYS2 project) 16.2.0`.
  - They import libgcc_s_seh-1.dll, libstdc++-6.dll and msvcrt.dll.
  - qpdf30.dll contains the paths `D:/a/qpdf/qpdf/vcpkg/installed/x64-mingw-static/...`.
- Libraries statically linked inside qpdf30.dll:
  - OpenSSL 3 (vcpkg static)
  - zlib 1.3.2 (string "deflate 1.3.2 Copyright 1995-2026 Jean-loup Gailly and Mark Adler")
  - libjpeg-turbo 3.2.0
- DLLs the choco build brings into `resources\`:
  - qpdf30.dll
  - libgcc_s_seh-1.dll (d7e343dd…)
  - libstdc%2B%2B-6.dll (494bd904…)
  - libwinpthread-1.dll (72bf8802…)
  - These GCC runtime copies differ from the ones in ocr/bin (different hashes), so they are separate builds.
  - The globs match case-insensitively, so they are covered by the GCC runtime and winpthreads entries.
- Mismatches:
  - The GCC runtime entry's note says these libraries are "for the Windows PDF and OCR helpers" with sources at repo.msys2.org. The qpdf copies come from qpdf's own GitHub CI toolchain (MSYS2 GCC 16.2.0 Rev4).
  - The qpdf entry does not say that its Windows binary statically includes OpenSSL, zlib and libjpeg-turbo. Those components do have entries of their own, but only DLL and dylib globs.
  - The MSVC Redistributable note applies only to the GitHub-installer (msvc64) path. The MSIX ships no vcruntime.
- License: Apache-2.0 is correct.
  - Upstream NOTICE.md at v12.4.2: "Versions of qpdf prior to version 7 were released under ... Artistic License [2.0]. At your option, you may continue to consider qpdf to be licensed under those terms." This is an optional alternative. It does not make the license a required dual.
- **Copyright mismatch.**
  - Upstream text (NOTICE.md, and the `--copyright` strings in the binary): `Copyright (c) 2005-2021 Jay Berkenbilt` and `Copyright (c) 2022-2026 Jay Berkenbilt and Manfred Holger`.
  - The JSON has "2005-2026 Jay Berkenbilt and the qpdf contributors".
- **NOTICE not reproduced.**
  - qpdf ships NOTICE.md (https://github.com/qpdf/qpdf/blob/v12.4.2/NOTICE.md). It contains the qtest Artistic-2.0 note, the public-domain Rijndael code, and the **sphlib 3.0 MIT licence (Copyright (c) 2007-2011 Projet RNRT SAPHIR)**.
  - THIRD-PARTY-NOTICES.txt contains none of these: grep for SAPHIR or sphlib finds nothing.
  - Apache-2.0 §4(d) requires redistributing the NOTICE content, and the MIT terms require keeping sphlib's notice.

## 2. macOS (bundle-qpdf.sh)

- Workflows run `brew install qpdf poppler tesseract` with no version pin. bundle-qpdf.sh accepts any qpdf ≥ 11.0.0.
- The script copies these executables plus their otool non-system dylib closure:
  - qpdf into Resources/qpdf-bin
  - pdftoppm and pdftotext into Resources/pdf-bin
  - tesseract into Resources/resources/ocr/bin
- Closure on this machine (formula, version, dylib):
  - qpdf 12.4.1: libqpdf.30; jpeg-turbo 3.2.0 libjpeg.8; openssl@3 3.6.3 libcrypto.3.
  - poppler 26.04.0:
    - libpoppler.159
    - fontconfig 2.18.3, freetype 2.14.3, gettext 1.0 libintl.8
    - **gpgme 2.1.0 libgpgme.45**, **gpgmepp 2.1.0 libgpgmepp.7**, **libassuan 3.0.2 libassuan.9**, **libgpg-error 1.61 libgpg-error.0**
    - jpeg-turbo, libpng 1.6.59, libtiff 4.7.2, little-cms2 2.19
    - **nspr 4.39 libnspr4/libplc4/libplds4**
    - **nss 3.124 libnss3/libnssutil3/libsmime3/libssl3**
    - openjpeg 2.5.4, webp 1.6.0 (libwebp, libsharpyuv), xz 5.8.4 liblzma, zstd 1.5.7
  - tesseract 5.5.3: libtesseract.5, leptonica 1.87.0, giflib 6.1.3, libarchive 3.8.9, libb2 0.98.1, lz4 1.10.0, libpng, libtiff, jpeg-turbo, openjpeg, webp (incl. libwebpmux), xz, zstd.
  - Not in the closure: cairo, pango, glib, harfbuzz, icu, gnutls, nettle, gmp. They are brew dependencies but are not linked by these binaries.
- **Dylibs with no glob.** `--binaries` would fail on these (globMatches is case-insensitive, `*` matches any characters):
  - libgpgme.45.dylib, libgpgmepp.7.dylib: LGPL-2.1-or-later
  - libassuan.9.dylib: Homebrew lists "LGPL-2.1-or-later AND GPL-3.0-or-later AND FSFULLR"
  - libgpg-error.0.dylib: LGPL-2.1-or-later
  - libnss3, libnssutil3, libsmime3: MPL-2.0
  - libnspr4, libplc4, libplds4: MPL-2.0
- **Wrong attribution:** NSS's libssl3.dylib (MPL-2.0) matches the OpenSSL glob `libssl*.dylib`.
- Other differences:
  - Poppler source URL: the JSON points to the 26.02.0 tarball. macOS uses Homebrew's current poppler (26.04.0 installed here; 26.10.0 in the API).
  - Poppler license: Homebrew says "GPL-2.0-only OR GPL-3.0-only"; the JSON says GPL-2.0-or-later.
  - fontconfig license: Homebrew says "HPND-sell-variant AND Unicode-3.0 AND MIT-Modern-Variant AND MIT AND public-domain"; the JSON says MIT.
- Corresponding source:
  - Each bottle is built from the formula's stable URL and sha256 at a specific homebrew-core commit, plus any formula patches. openjpeg has 2 patches and libb2 has 1.
  - Identifiers would be the INSTALL_RECEIPT.json of each keg (`source.tap_git_head`, `source.versions`, `revision`, bottle rebuild), or `brew info --json=v2 --installed` captured at build time. CI records none of these.

## 3. Fonts and models

- **DejaVu Sans 2.37**:
  - The font's sha256 7da195a7… matches manifest.json.
  - The repo LICENSE.txt matches the upstream version_2_37 LICENSE (diff is empty).
  - The font's name-table copyright ends "DejaVu changes are in public domain". The JSON copyright omits that phrase.
  - "Arev" is not an SPDX ID (only Bitstream-Vera is on the SPDX list), so it would need a LicenseRef.
- **Satisfy**:
  - The font is byte-identical to google/fonts `apache/satisfy`, and its licence file is identical to upstream LICENSE.txt.
  - The licence is Apache-2.0, which is correct.
  - **Copyright mismatch:** the font and METADATA.pb say `Copyright (c) 2011 by Font Diner, Inc. All rights reserved.`, plus the trademark notice "Satisfy is a trademark of Font Diner, Inc.". The JSON has "the Satisfy project authors".
- **Pacifico**:
  - The font is byte-identical to google/fonts. OFL-1.1 and "Copyright 2018 The Pacifico Project Authors" are correct. The URL suffix is missing from the JSON copyright.
  - The licence file differs from upstream OFL.txt only by trailing whitespace on 2 lines.
  - **Version mismatch:** the font says "Version 3.001"; the JSON and manifest say 7.003.
- **YuNet**:
  - face_detection_yunet_2023mar.onnx sha256 8f2383e4… matches opencv_zoo main.
  - The directory LICENSE is MIT, "Copyright (c) 2020 Shiqi Yu". This matches the JSON and the licence file.
- **U2-Net u2netp**:
  - The upstream repo licence is Apache-2.0, but its LICENSE has an unfilled template line (`Copyright [yyyy] [name of copyright owner]`). The README has no separate licence statement for the weights, so the JSON copyright line is not stated upstream.
  - The shipped u2netp.onnx (4,574,861 bytes) matches the size of danielgatis/rembg release v0.0.0 `u2netp.onnx`. rembg is MIT, "Copyright (c) 2020 Daniel Gatis". Upstream U-2-Net ships only a .pth file.
  - The JSON names neither rembg nor the conversion.
