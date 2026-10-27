# Tesseract / MSYS2 DLL license verification (MSIX 1.0.17, read-only)

Facts, not legal conclusions. Date checked: 2026-10-22.

## 1. Provenance and versions

- Workflow: `.github/workflows/microsoft-store.yml:112` runs `choco install qpdf tesseract -y`. No `--version`, so it is not pinned. `scripts/stage-ocr-resources.mjs` copies tesseract.exe plus **every** `*.dll` in the Tesseract-OCR folder.
- The current Chocolatey package is tesseract **5.5.3.20260724** (approved 2026-07-25), https://community.chocolatey.org/packages/tesseract. The previous version was 5.5.0.20241111. The package bundles `tesseract-ocr-w64-setup-5.5.3.20260724.exe` (sha256 BEE9E343...0AE4), taken from https://github.com/tesseract-ocr/tesseract/releases/tag/5.5.3 (published 2026-07-24). That release is the official tesseract-ocr one, not a UB-Mannheim GitHub release; the last UB-Mannheim release is v5.4.0.20240606.
- The shipped binaries match it. libtesseract-5.dll contains `tesseract v5.5.3.20260724` and the build path `/home/debian/src/github/tesseract-ocr/tesseract/bin/ndebug/x86_64-w64-mingw32-5.5.3.20260724`.
- Build recipe: https://github.com/tesseract-ocr/tesseract/blob/main/nsis/build.sh and .github/workflows/installer-for-windows.yml (ubuntu-24.04, workflow_dispatch). Tesseract is cross-compiled with the Debian/Ubuntu `g++-mingw-w64` toolchain. The script runs `pacman -Syu` and installs the then-current MSYS2 packages (curl-winssl, giflib, icu, leptonica, libarchive, libidn2, openjpeg2, openssl, pango, libpng, libtiff, libwebp), copies their dependency DLLs (`find_deps.py`), and copies libstdc++-6/libgcc_s_seh-1 from `/usr/lib/gcc/x86_64-w64-mingw32/*/`. No package lock or version manifest is published. The `v5.5.3.20260724` tag does not exist on GitHub (404), because the script tags it locally.
- The GCC runtime is **not MSYS2**. libgcc_s_seh-1.dll and libstdc++-6.dll show `GCC: (GNU) 14`, `GNU C17 14-posix` and `./build/x86_64-w64-mingw32-posix/`, which means Debian/Ubuntu gcc-mingw-w64 (GCC 14). The exact Debian package revision is not recorded. tesseract.exe and libtesseract were built with the same toolchain. libwinpthread-1 is MSYS2 (GCC 16.1.0 Rev5).

Versions. Strings and PE version resources are confirmed from the binaries. Where no string was found, the version was inferred from the MSYS2 MINGW-packages PKGBUILD as of 2026-07-24 (marked *):

| DLL | version | DLL | version |
|---|---|---|---|
| tesseract | 5.5.3 (20260724) | leptonica | 1.87.0-1* |
| glib/gio/gobject/gmodule | 2.88.2 | pango (4 dlls) | 1.58.0 |
| harfbuzz | 14.2.1 | fribidi | 1.0.16 |
| graphite2 | 1.3.15-1* | libthai / libdatrie | 0.1.30-1* / 0.2.14-1* |
| giflib | 6.1.3-1* | libidn2 | 2.3.8 |
| libunistring | 1.4.2 | gettext libintl | 1.0 |
| libiconv | 1.19 | libffi | 3.7.1 |
| pcre2 | 10.47 | ICU | 78.3 |
| libarchive | 3.8.8 | libb2 | 0.98.1-3* |
| brotli | 1.2.0-1* | bzip2 | 1.0.8 |
| lz4 | 1.10.0 | xz/liblzma | 5.8.3 |
| libpsl | 0.21.5 | libwebp/mux/sharpyuv | 1.6.0-1* |
| curl (winssl) | 8.21.0 | libssh2 | 1.11.1 |
| libtiff | 4.7.2 | libjpeg-turbo | 3.2.0 |
| libpng | 1.6.58 | zlib | 1.3.2 |
| zstd | 1.5.7 | openjpeg | 2.5.4 |
| freetype | 2.14.3 | expat | 2.8.2 |
| cairo | 1.18.4 | pixman | 0.46.4 |
| fontconfig | 2.18.2-1* | libdeflate / Lerc | 1.25* / 4.1.1* |
| JBIG-KIT | 2.1 (MSYS2 2.1-5*) | winpthreads | MSYS2 14.0.0.r220* |

- The MSYS2 source packages for these versions currently exist at https://repo.msys2.org/mingw/sources/, for example mingw-w64-jbigkit-2.1-5, leptonica-1.87.0-1, glib2-2.88.2-1 and curl-8.21.0-2 (HTTP 200). MSYS2 prunes old sources, though, and the pkgrel values marked * are inferences. The exact corresponding source is identifiable only by this reconstruction, not from a manifest. Debian's gcc-mingw-w64 source is a different origin from what the notice states.
- No libcrypto or libssl is shipped in ocr/bin, because curl is the winssl build.
- **Unpinned:** any CI run after a new Chocolatey or upstream release will ship different binaries and versions.
- **Shipped but not loaded by tesseract.exe** (only Tesseract's training tools use them): pango*, cairo, pixman, fontconfig, freetype, harfbuzz, graphite2, glib/gio/gobject/gmodule, pcre2, ffi, fribidi, thai, datrie, icu* (from `objdump -p` import tables).

## 2. GPL-only components

- libjbig-0.dll is present. It contains `JBIG-KIT 2.1 -- (c) 1995-2014 Markus Kuhn -- Licence: GPL`. libtiff-6.dll imports libjbig-0.dll, and tesseract.exe and libleptonica both import libtiff-6, so libjbig is loaded on every OCR run. The JBIG codec itself runs only on JBIG-compressed TIFF input.
- Upstream (https://www.cl.cam.ac.uk/~mgk25/jbigkit/) says GPL, links GPLv2, and offers a commercial licence. Debian copyright records `GPL-2+` (https://metadata.ftp-master.debian.org/changelogs/main/j/jbigkit/unstable_copyright). MSYS2 tags `spdx:GPL-2.0`. Upstream released 2.2 on 2026-10-09; 2.1 is shipped.
- The obligations for distributing a GPL binary are: provide the complete corresponding source (or a written offer), the license text, and the notices. The repo has GPL-2.0-only.txt but no GPL-2.0-or-later text. It is the same text, but check how the notice generator maps the id.
- No other DLL in the set is GPL-only. libidn2 and libunistring are LGPL-3+/GPL-2+. Graphite2 is a triple license. FreeType is FTL/GPL-2+. The libiconv library is LGPL-2.1+.

## 3. Mismatches against native-components.json

- **All MSYS2/OCR entries:** no `version` (only JBIG-KIT has one), and the source URLs are generic project pages rather than version-specific tarballs or MSYS2 src packages.
- **Tesseract:** the note says "UB Mannheim / MSYS2 build" and links UB-Mannheim/tesseract. The actual binary is the official tesseract-ocr 5.5.3 installer, cross-built with Debian mingw. The copyright omits Hewlett-Packard ("(C) Copyright 1992, Hewlett-Packard Ltd." in source headers). Version 5.5.3 is missing.
- **Leptonica:** the upstream license file reads "Copyright (C) 2001-2020 Leptonica"; the entry says 2001-2024 (minor). Version 1.87.0 is missing.
- **Pango:** COPYING is the GNU *Library* GPL v2 and the headers say "version 2 ... or any later". That is LGPL-2.0-or-later, but the entry says LGPL-2.1-or-later. The holders are Red Hat Software and others, not "the Pango contributors".
- **HarfBuzz:** upstream calls it the "Old MIT" license (SPDX MIT-Modern-Variant), not MIT, and lists named holders (Google, Behdad Esfahbod, Red Hat, ...).
- **libthai and libdatrie:** a combined entry with only the libthai source URL. https://github.com/tlwg/libdatrie is missing.
- **winpthreads:** upstream COPYING holders are "Copyright (c) 2011 mingw-w64 project" (MIT) and "(C) 2010 Lockless Inc." (BSD-3-Clause). The entry's "John E. Bossom / pthreads-win32 contributors" text does not appear in the upstream COPYING.
- **GCC runtime:** the note says the sources are at repo.msys2.org. The shipped libgcc_s and libstdc++ come from Debian/Ubuntu gcc-mingw-w64 (GCC 14), so the source is the Debian gcc-mingw-w64 package. Version missing.
- **PCRE2:** upstream LICENCE lists "1997-2007 University of Cambridge" and "2007-2024 Philip Hazel" (plus JIT authors). The entry lists only Cambridge 1997-2024.
- **libffi:** upstream reads 1996-2026; the entry says 1996-2024 (minor).
- **ICU:** the upstream LICENSE contains "Copyright © 1991-2026 Unicode, Inc." and bundled third-party licenses; the entry says 2016-2025.
- **libpsl:** upstream reads "Copyright (C) Tim Rühsen and Libpsl Contributors"; the entry says "2014-2024 Tim Ruehsen" (minor).
- **libarchive:** upstream is BSD-2-Clause, plus some UC Regents BSD-3 files, public-domain files and CC0/OpenSSL/Apache BLAKE2 files. The entry says BSD-2-Clause only.
- **GLib, FriBidi, Graphite2, giflib (MIT), libidn2, libunistring, gettext, libiconv, Brotli, bzip2, LZ4, XZ (0BSD), libb2 (CC0-1.0), libwebp:** the license ids match upstream. Versions are missing. The FriBidi holders are named people (Behdad Esfahbod, Dov Grobgeld, Roozbeh Pournader), not "the FriBidi contributors".
- **JBIG-KIT:** the entry (2.1, GPL-2.0-or-later, Markus Kuhn) matches. Its source URL points to the upstream tarball, not the MSYS2 src package that was actually built.
