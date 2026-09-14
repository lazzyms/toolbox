# Gates: PDF editor delivery

- [x] G1: Both branches are reverified and integrated
  CHECK: node /Users/mauliksompura/.codex/skills/unlazy/scripts/gate-check.mjs --root . --cwd . --reverify .unlazy/pdf-scene/gates/node-1.1.md .unlazy/pdf-scene/gates/node-1.2.md
  EXPECT: ALL MET
  EVIDENCE: automatic-evidence=v1; definition-sha256=f45116eac23f1b7444dd61836bcdb445359b9af0c6be5f9330e0f95b0b513d3f; exit=0; EXPECT=matched; output-sha256=c10180463974dbbe82c7c334ca7b988af49f4e897a2d19f0b21762d9a0f16a39; output-bytes=4493; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox; path=49172725d4c9/48 entries

- [x] G2: Native command matrix remains green
  CHECK: npm run test:native:e2e && echo PDF_SCENE_E2E_OK
  EXPECT: PDF_SCENE_E2E_OK
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=bd226e36117e5a0643188a59b5588b39cae18df22690252de7752ef68c48bb14; exit=0; EXPECT=matched; output-sha256=c7d1278da8f903f7605c92eea2fd309208a69502ef4a8b2939dffb9cea8b60d7; output-bytes=547; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=49172725d4c9/48 entries

- [x] G3: Synthetic mixed-edit document verified visually in exact dev app and exported PDF, with preview above fold
  EVIDENCE: 2026-09-10 exact debug bundle process /ToolboxTauri/src-tauri/target/debug/bundle/macos/Toolbox.app; synthetic document displayed above fold with five simultaneous objects; one export opened in Preview as document-edited-2.pdf with text, highlight, shape, watermark and signature; source SHA-256 remained 403b8da1ea37d6faa7f74cd75b7f98d58ecef267be4762f10504997b237ffc4a
