# Gates: 1.2 integration

- [x] N1: Named children pass independent re-verification
  CHECK: node /Users/mauliksompura/.codex/skills/unlazy/scripts/gate-check.mjs --root . --cwd . --reverify .unlazy/pdf-scene/gates/leaf-1.2.1.md
  EXPECT: ALL MET
  EVIDENCE: automatic-evidence=v1; definition-sha256=e2d04efc23768bd1f0ca14c405b468657465001c12b0d7dc50e43dfa1707758d; exit=0; EXPECT=matched; output-sha256=2e6a8e02304690da4b3028003cf9196247360d06da1f004ef4c5968c57ce7f97; output-bytes=2371; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox; path=49172725d4c9/48 entries

- [x] N2: All workspace routes and existing regressions pass
  CHECK: npm run test:ui && echo PDF_SCENE_REGRESSION_OK
  EXPECT: PDF_SCENE_REGRESSION_OK
  CWD: ToolboxTauri
  EVIDENCE: automatic-evidence=v1; definition-sha256=4ca44398ad6c3ce87caad30503130ce25669ca4d089d3e7062a9f6e0bd8a0d76; exit=0; EXPECT=matched; output-sha256=233f648ae926d088ae1e29d5e3c64ad947c44ac666c78cd1bef8d6376f80f47e; output-bytes=9613; shell=/bin/sh; cwd=/Users/mauliksompura/Documents/Projects/Personal/toolbox/ToolboxTauri; path=49172725d4c9/48 entries

- [x] N5: Child leases released after re-verification
  EVIDENCE: leaf-1.2.1 released after parent re-verification; recorded in status.log
