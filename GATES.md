# Gates: Toolbox workspace consolidation

OWNS: ToolboxTauri/src/**, ToolboxTauri/src-tauri/src/**, ToolboxTauri/tests/**, ToolboxTauri/scripts/**, skills/verify-toolbox/features/**, GATES.md

Scope: Complete the seven requested consolidation passes in the Tauri app without changing the 32 atomic tool IDs, leaving the app local-only and preserving source-file safety.

- [x] G0: the consolidation ledger is structurally valid
  CHECK: node /Users/mauliksompura/.codex/skills/unlazy/scripts/gate-lint.mjs GATES.md
  EXPECT: LINT OK
  EVIDENCE: automatic-evidence=v1; definition-sha256=15230cded0c216e2ec467af163001532e0f206f68948f4b6a4efbef7eef41f54; exit=0; EXPECT=matched; output-sha256=2c557a6113db4c377941a81dddd5af21c7a575967db82d8350da20817bb50659; output-bytes=151; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox; path=9c390fa4736f/48 entries

- [x] G1: the pre-change registry and release contract remain valid
  CHECK: npm run check:release
  EXPECT: Release configuration passed
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=39d238eaddb7422121c6637277ba4d1b162cb05cc5ffa0d81dc13b13bc04651f; exit=0; EXPECT=matched; output-sha256=6bbaad41cd3a5a7150e74de679f9e90bec2d32e42e95e7a060032ff40f82689b; output-bytes=294; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=9c390fa4736f/48 entries

- [x] G2: the pre-change application builds
  CHECK: npm run build && node -e "console.log('BUILD_BASELINE_GATE_PASSED')"
  EXPECT: BUILD_BASELINE_GATE_PASSED
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=39aadf02b530cc59cc5b79a0e276c527d10af194539fdaab5d706007d1599d10; exit=0; EXPECT=matched; output-sha256=d768d31ccd63dc64999ef89e71922e023ba473428656a4d70b9bd2c6d38db133; output-bytes=478; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=9c390fa4736f/48 entries

- [x] G3: the pre-change browser surface is green
  CHECK: npm run test:ui && node -e "console.log('UI_BASELINE_GATE_PASSED')"
  EXPECT: UI_BASELINE_GATE_PASSED
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=317dffb46c8df0cd1d11aa1ff85db99a1635d8a5f098ab6654cfa49a9ae8ddd2; exit=0; EXPECT=matched; output-sha256=3f60ced3276a6d045680e24eacfc058c332a40722b46b277e4ea77066091d56a; output-bytes=8840; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=9c390fa4736f/48 entries

- [x] G4: the pre-change native suite is green
  CHECK: cargo test --manifest-path src-tauri/Cargo.toml && node -e "console.log('NATIVE_BASELINE_GATE_PASSED')"
  EXPECT: NATIVE_BASELINE_GATE_PASSED
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=bbe03b68b730a70b59dc13819e20e8dc87c8f7cb4623b8b21f87e796a938777f; exit=0; EXPECT=matched; output-sha256=65b0e8c82d5c183951437620a4c9d6a70e162507c0d634a5f904441d8c244656; output-bytes=5770; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=9c390fa4736f/48 entries

- [x] G5: the PDF editor supports viewer-grade session editing and one composed export
  CHECK: npm run test:ui -- tests/ui/feature-navigation.spec.ts -g "PDF editor" && node -e "console.log('PDF_EDITOR_GATE_PASSED')"
  EXPECT: PDF_EDITOR_GATE_PASSED
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=a9d7d6cb91ed79a696aaf05432a7a13864158170f5da28ab94d4aebaee2a93c8; exit=0; EXPECT=matched; output-sha256=ab8c1b31f1cec766814d68f2d5aa298153fb2ab6523d8a85be0cf41428a5844f; output-bytes=1479; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=9c390fa4736f/48 entries

- [x] G6: the image editor previews a typed non-destructive edit plan and exports it once
  CHECK: npm run test:ui -- tests/ui/feature-navigation.spec.ts -g "Image editor" && node -e "console.log('IMAGE_EDITOR_GATE_PASSED')"
  EXPECT: IMAGE_EDITOR_GATE_PASSED
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=039ddac19c3bfc6a819f387359aa2fb808db648bd5bd789e929aecc48f18db7a; exit=0; EXPECT=matched; output-sha256=559e84d4cbcc2049cf10ab118a8fbec725030651e21a2f826cf15ee44b75166d; output-bytes=958; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=9c390fa4736f/48 entries

- [x] G7: PDF conversion exposes page-scoped outputs and OCR in the conversion workspace
  CHECK: npm run test:ui -- tests/ui/feature-navigation.spec.ts -g "PDF conversion" && node -e "console.log('PDF_CONVERSION_GATE_PASSED')"
  EXPECT: PDF_CONVERSION_GATE_PASSED
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=daa3d57bab8454fdca1b90ef4ff786d4d93b420fae6fda0d0c400691a4949e62; exit=0; EXPECT=matched; output-sha256=9128c93af0772aaf8f95d90fcba68da5c49b75771e867b112493ab070f34596f; output-bytes=652; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=9c390fa4736f/48 entries

- [x] G8: media, security, and privacy actions share their intended file-selection and outcome contracts
  CHECK: npm run test:ui -- tests/ui/feature-navigation.spec.ts -g "media|security|metadata|vision" && node -e "console.log('MEDIA_SECURITY_GATE_PASSED')"
  EXPECT: MEDIA_SECURITY_GATE_PASSED
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=976963c9c9fe3bb3b1aee7e44a84caf0c0b466513203c0f633c26c0f7ecaeda2; exit=0; EXPECT=matched; output-sha256=ab1623831b4ca757fb25906adc161fae9b02081bc0350f3415467bf57a81c767; output-bytes=1278; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=9c390fa4736f/48 entries

- [x] G9: the shared workspace framework owns action routing and no obsolete standalone view is reachable
  CHECK: node scripts/check-workspace-consolidation.mjs && node -e "console.log('WORKSPACE_FRAMEWORK_GATE_PASSED')"
  EXPECT: WORKSPACE_FRAMEWORK_GATE_PASSED
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=259661c85697866de044bf214b320e041720e542bdda6391ddcd681d216b7192; exit=0; EXPECT=matched; output-sha256=6e1093ecb763e4baa4242711f807b6828ff356b3a38952a6ded46abc8ef22bf8; output-bytes=119; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=9c390fa4736f/48 entries

- [x] G10: all native commands and the fixture-backed workspace path remain green after integration
  CHECK: npm run test:native:e2e && node -e "console.log('NATIVE_INTEGRATION_GATE_PASSED')"
  EXPECT: NATIVE_INTEGRATION_GATE_PASSED
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=d6d6194b78c6f946c6200814efa603544fbbcd8e64ecaa3a40a370f3eef5909f; exit=0; EXPECT=matched; output-sha256=ed24f4da6186c295b4aa5f8f0d0d64d737f6d483769e35b7e455b4b1211ee346; output-bytes=561; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=9c390fa4736f/48 entries

- [ ] G11: the actual development Tauri app is verified through the user-facing desktop surface
  EVIDENCE: pending
