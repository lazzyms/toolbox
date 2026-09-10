# Gates: 1.1 integration

- [x] N1: Named children pass independent re-verification
  CHECK: node /Users/mauliksompura/.codex/skills/unlazy/scripts/gate-check.mjs --root . --cwd . --reverify .unlazy/pdf-scene/gates/leaf-1.1.1.md .unlazy/pdf-scene/gates/leaf-1.1.2.md
  EXPECT: ALL MET
  EVIDENCE: automatic-evidence=v1; definition-sha256=d7e86d7e3afa7112b5ea1ae699657ee069c73df3bcf9e6049fafbc646c050db6; exit=0; EXPECT=matched; output-sha256=d867ef11fd9e1cc2e6c9710cb01b33e0b6f099a1cfcb9f180832ff0421904a54; output-bytes=2438; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox; path=49172725d4c9/48 entries

- [x] N2: Native and canvas schemas compile together
  CHECK: npm run build && echo PDF_SCENE_BUILD_OK
  EXPECT: PDF_SCENE_BUILD_OK
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=2c64f2f288984ff6eb219a5e7c4bbc15dce95a3cb03d210c5e62590a5f947e63; exit=0; EXPECT=matched; output-sha256=8c040084e699a513769a7341789d2c805713ddf080824573e9016acc2dbf6fd6; output-bytes=470; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=49172725d4c9/48 entries

- [x] N5: Child leases released after re-verification
  EVIDENCE: leaf-1.1.1 and leaf-1.1.2 released after parent re-verification; recorded in status.log
