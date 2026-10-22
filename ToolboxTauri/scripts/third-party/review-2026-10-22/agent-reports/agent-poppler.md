# Poppler-windows 26.02.0-0 license verification (read-only)

Date: 2026-10-22. Zip SHA256 pinned in .github/workflows/microsoft-store.yml:119 (993e4a94...). Workflow copies only pdftoppm.exe, pdftotext.exe and every *.dll in Library/bin (lines 129-140). share/poppler (poppler-data), man pages, .lib, headers, other pdf*.exe and zstd.exe are NOT shipped.

## 1. How the release is built

- Repo at tag: https://github.com/oschwartz10612/poppler-windows/tree/v26.02.0-0 (commit a6703d8f, repo license MIT). Files: package.sh, .github/workflows/release.yaml, README.md.
- release.yaml: `conda install -c conda-forge liblzma expat poppler -y` on windows-latest via setup-miniconda. NO environment file, NO version/build pins, NO lockfile. package.sh then `cp`s DLLs out of conda_pkgs_dir/<pkg>*/Library/bin and downloads poppler-data-0.4.12.
- It is a repackage of conda-forge binaries (README says so: built from https://github.com/conda-forge/poppler-feedstock).
- Run log: https://github.com/oschwartz10612/poppler-windows/actions/runs/25300974086 (2026-05-04). Job log returns HTTP 410 (expired), so the solved package list is NOT recorded anywhere public. Versions below are reconstructed from DLL version resources, conda `D:\bld\<pkg>_<epoch-ms>` paths embedded in DLLs, and https://api.anaconda.org/package/conda-forge/<name>/files (win-64 uploads before 2026-05-04 04:25Z).

| DLL | conda-forge pkg | version / build (evidence) | conda license |
|---|---|---|---|
| poppler*.dll, pdftoppm/pdftotext | poppler (poppler-split) | 26.02.0 h4b9d284_3 (bld path poppler-split_1777029927371 = 2026-04-24; upload 2026-04-24 11:43) — CONFIRMED | GPL-2.0-or-later |
| cairo.dll | cairo | 1.18.4, likely h477c42c_1 (only build before cutoff after 2025-12) — INFERRED | LGPL-2.1-only or MPL-1.1 |
| pixman-1-0.dll | pixman | 0.46.4 h5112557_1 — INFERRED | MIT |
| fontconfig-1.dll | fontconfig | 2.17.1 hd47e2ca_0 (bld fontconfig_1771382559288 = 2026-02-18) — CONFIRMED | MIT |
| freetype.dll | libfreetype6 | 2.14.3 (PE version) hdbac1cb_0 | GPL-2.0-only OR FTL |
| expat.dll, libexpat.dll (same file copied twice) | expat/libexpat | 2.8.0 (PE) hac47afa_0 | MIT |
| jpeg8.dll | libjpeg-turbo | 3.1.4.1 (PE) hfd05255_0 | IJG AND BSD-3-Clause AND Zlib |
| libpng16.dll | libpng | 1.6.58 (string) h7351971_0 | zlib-acknowledgement |
| tiff.dll, libtiff.dll | libtiff | 4.7.1 (PE) h8f73337_1 (bld libtiff_1762021852211 = 2025-11-01) — CONFIRMED | HPND |
| openjp2.dll | openjpeg | 2.5.4 (string), likely h0e57b4f_0 | BSD-2-Clause |
| lcms2.dll | lcms2 | 2.19 (PE) hf2c6c5f_0 (bld lcms2_1777250392545 = 2026-04-27) — CONFIRMED | MIT |
| zlib.dll | libzlib | 1.3.2 (PE) hfd05255_2 | Zlib |
| zstd.dll, libzstd.dll | zstd | 1.5.7 (PE) h534d264_6 (zip mtime 2025-12-03) | BSD-3-Clause |
| Lerc.dll | lerc | 4.1.0 hd936e49_0 — INFERRED | Apache-2.0 |
| deflate.dll | libdeflate | 1.25 h51727cc_0 — INFERRED | MIT |
| libcurl.dll | libcurl | 8.20.0 (PE) h8206538_0 | curl |
| libssh2.dll | libssh2 | 1.11.1 (PE) h9aa295b_0 (bld libssh2-split_1745608374449 = 2025-04-25) — CONFIRMED | BSD-3-Clause |
| libcrypto-3-x64.dll | openssl | 3.6.2 (PE "OpenSSL 3.6.2 7 Apr 2026") hf411b9b_0 (bld openssl_split_1775587024900) — CONFIRMED | Apache-2.0 |
| iconv.dll, charset.dll | libiconv | 1.18 hc1393d2_2 (zip mtime 2025-08-11) | LGPL-2.1-only (conda metadata) |
| liblzma.dll | liblzma | 5.8.3 (PE) hfd05255_0 | 0BSD |

Corresponding source is identifiable but not pinned by the packager:
- Poppler build 3 = poppler-feedstock commit b46a81fb3d74 (2026-04-24, "Rebuild for libboost 1.90"): https://github.com/conda-forge/poppler-feedstock/tree/b46a81fb3d74/recipe — source https://poppler.freedesktop.org/poppler-26.02.0.tar.xz sha256 dded8621f7b2f695c91063aab1558691c8418374cd583501e89ed39487e7ab77, Windows patches exportsymbols.patch, windows-data.patch, disable-libtiff-win32-io.patch, includesystembeforejpeg.patch, plus bld.bat / conda_build_config.yaml. Each conda .conda package also carries info/recipe and info/licenses, but package.sh copies only Library/, so they are dropped.
- Other DLLs: corresponding feedstock (github.com/conda-forge/<name>-feedstock) at the commit matching the build string; not recorded by the packager.

Zip contents of license material: only poppler-26.02.0/share/poppler/{COPYING, COPYING.adobe, COPYING.gpl2, README} — these belong to poppler-data 0.4.12 (README says so), not Poppler itself. No Poppler COPYING/COPYING3, no dependency license files, no source. None of these are shipped by the workflow anyway.

## 2. License / copyright checks

Poppler 26.02.0 (curl of https://gitlab.freedesktop.org/poppler/poppler/-/raw/poppler-26.02.0/{README.md,README-XPDF,COPYING,COPYING3,poppler/PDFDoc.cc}):
- README.md: "Poppler is licensed under the GPL"; ships COPYING (GPLv2) AND COPYING3 (GPLv3).
- Source headers: "All changes made under the Poppler project to this file are licensed under GPL version 2 or later", on top of Glyph & Cog Xpdf code.
- README-XPDF: Xpdf is "GPL, version 2 or 3" and explicitly NOT "any later version".
- => Effective license of the combined code is GPL-2.0-only OR GPL-3.0-only. native-components.json says GPL-2.0-or-later (also conda's metadata) — MISMATCH. Notices should carry both GPL-2.0 and GPL-3.0 texts.

Comparison with native-components.json (all files covered by globs, checked with the script's own case-insensitive regex):

| Component | Notice | Upstream | Mismatch |
|---|---|---|---|
| Poppler | GPL-2.0-or-later | GPL-2.0-only OR GPL-3.0-only (Xpdf) | license id; version 26.02.0 only in source URL, no "version" field; note links release page, not feedstock commit/patches |
| cairo | LGPL-2.1-only OR MPL-1.1 | same | no version (1.18.4); generic source URL |
| pixman | MIT, "the pixman contributors" | MIT, long holder list in COPYING (https://gitlab.freedesktop.org/pixman/pixman/-/raw/pixman-0.46.4/COPYING) | copyright generic; no version 0.46.4 |
| Fontconfig | MIT, "2000-2007 Keith Packard and contributors" | HPND-sell-variant-style text (not MIT wording); holders Keith Packard, Patrick Lam, Dwayne Bailey, Roozbeh Pournader, Red Hat 2008-2020, Danilo Segan, Google | license id (SPDX "HPND-sell-variant" fits better); incomplete copyright; no version 2.17.1 |
| FreeType | FTL OR GPL-2.0-or-later | LICENSE.TXT: GPLv2 "(any later version can be used also)" — OK; conda says GPL-2.0-only | none on id; no version 2.14.3 |
| Expat | MIT, 1998-2000 TOSSC/Clark Cooper; 2001-2025 maintainers | matches R_2_8_0 expat/COPYING | no version 2.8.0 |
| libjpeg-turbo | IJG AND BSD-3-Clause AND Zlib | matches | no version 3.1.4.1 |
| libpng | libpng-2.0, "1995-2025 The PNG Reference Library Authors" | 1.6.58 LICENSE: 1995-2026 Authors; 2018-2026 Cosmin Truta; 2000-2018 Glenn Randers-Pehrson; 1996-1997 Andreas Dilger; 1995-1996 Guy Eric Schalnat, Group 42 | copyright year and missing holders; http:// source; no version |
| LibTIFF | libtiff | matches LICENSE.md v4.7.1 | no version |
| OpenJPEG | BSD-2-Clause, "the OpenJPEG contributors" | LICENSE lists UCL, Benoit Macq, Antonin Descampe, ... CNES, CS Systemes | copyright generic (BSD-2 requires retaining notice); no version 2.5.4 |
| Little CMS | MIT, 1998-2024 Marti Maria Saguer | LICENSE 2.19: "Copyright (c) 2023 Marti Maria Saguer"; PE says 2026 | year; no version 2.19 |
| zlib | Zlib, 1995-2024 | 1.3.2: (C) 1995-2026 | year; no version |
| Zstandard | BSD-3-Clause | BSD-3-Clause OR GPL-2.0-only (dual) | acceptable election, but id incomplete; no version |
| LERC | Apache-2.0, 2015 Esri | NOTICE: "Copyright 2015-2026 Esri" (Apache NOTICE file should be reproduced) | year; NOTICE text not included; no version 4.1.0 |
| libdeflate | MIT, 2016 Eric Biggers | COPYING v1.25: 2016 Eric Biggers; 2024 Google LLC | missing Google LLC; no version |
| curl | curl, 1996-2026 Stenberg | matches 8.20.0 | no version |
| libssh2 | BSD-3-Clause, "the libssh2 project" | COPYING lists Sara Golemon, Mikhail Gusarov, The Written Word, Eli Fant, Daniel Stenberg, Simon Josefsson, Markus Friedl, Microsoft | copyright generic (BSD-3 requires reproducing it); no version |
| OpenSSL | Apache-2.0, 1998-2025 | 3.6.2: 1998-2026 The OpenSSL Authors | year; no version |
| GNU libiconv | LGPL-2.1-or-later | upstream library LGPL-2.1-or-later; conda metadata LGPL-2.1-only | none on id (conda metadata disagrees); no version 1.18 |
| XZ (liblzma) | 0BSD | matches | no version 5.8.3 |

Unlisted binaries: none of the shipped files. Unlisted in zip but not shipped: pdfattach, pdfdetach, pdffonts, pdfimages, pdfinfo, pdfseparate, pdftocairo, pdftohtml, pdftops, pdfunite, zstd.exe.

Side observations (not license): from import strings, pdftoppm/pdftotext -> poppler.dll -> freetype, jpeg8, lcms2, libcurl(->libssh2->libcrypto), libpng16, openjp2, tiff(->Lerc, deflate, liblzma, zstd), zlib. cairo, pixman, fontconfig, expat/libexpat, iconv/charset, libtiff.dll, libzstd.dll, poppler-cpp.dll, poppler-glib.dll are copied but not used by the two tools; poppler-glib.dll imports glib-2.0-0/gobject/gio DLLs that are not in the zip, so it cannot load. Dropping them would shrink the LGPL surface (cairo, libiconv).

## 3. Obligations triggered (no legal conclusions)

GPL (Poppler: pdftoppm.exe, pdftotext.exe, poppler*.dll; FreeType/zstd only if GPL branch elected — not needed):
- Provide license text of GPLv2 and GPLv3 (both alternatives). Current notices: one license id, presumably GPL-2.0 text only.
- Offer complete corresponding source for the exact binaries: poppler-26.02.0 tarball + the four Windows patches + build scripts (bld.bat, conda_build_config.yaml, meta.yaml at feedstock commit b46a81fb3d74), via GPLv2 s3(a)/(b)/(c) or GPLv3 s6. Current notice links only the upstream tarball and the poppler-windows release page; no patches/build scripts, no written offer, and the GPLv2 s3(c) "pass along the upstream offer" route needs the upstream to have made one (conda-forge publishes source/recipes but poppler-windows makes no offer). Distributor hosting it (or a written offer) is the robust route.
- Source must remain available for the period required (GPLv2: 3 years for written offer; GPLv3 s6: as long as offering object code/spare parts).
- Binary that links Poppler (none here; tools run as separate processes from toolbox.exe).

LGPL (cairo LGPL-2.1, libiconv/charset LGPL-2.1+), dynamically linked DLLs:
- Ship license text (LGPL-2.1) and copyright notice — provided generically.
- Provide or offer corresponding source of the library (LGPL-2.1 s4/s6): exact cairo 1.18.4 / libiconv 1.18 sources plus conda patches. Current notice: generic release directory URLs only, no version, no patches.
- Allow the user to replace the DLL (dynamic linking satisfies technically; MSIX package files are read-only/signed — whether that impedes replacement is a question for counsel).

Permissive (BSD-2/BSD-3/MIT/HPND/libtiff/libpng/Zlib/curl/IJG/0BSD/Apache-2.0):
- Reproduce copyright notice + license text in distribution docs: OK where copyright text matches; incomplete for pixman, fontconfig, OpenJPEG, libssh2, libdeflate, libpng; stale years for zlib, OpenSSL, lcms2, LERC.
- Apache-2.0 (OpenSSL, LERC): include license; include NOTICE content where upstream has one (LERC NOTICE exists, not reproduced).
- IJG: state that software is based in part on the work of the IJG — present.
