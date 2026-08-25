# ADR 0002: Upscaling beyond bicubic

Status: proposed · Closes #35

## Context

Resize today resamples with CoreGraphics' high-quality filter, which is
bicubic/Lanczos-class: sharp, but it cannot invent detail. "AI upscale" asks
for synthesis. The question is what an offline, dependency-free Mac app can
honestly offer.

## Options considered

1. **CoreImage Lanczos + sharpen.** `CILanczosScaleTransform` followed by
   `CISharpenLanczos` gives visibly cleaner enlargements than plain bicubic.
   Deterministic, system framework, ships today.
2. **System ML upscalers (MetalFX, CoreML bundled models).** Real detail
   synthesis where available, but model availability varies by chip and OS
   version, and results are not reproducible across machines.
3. **Third-party models (Real-ESRGAN and friends).** Best quality; violates
   the ToolboxKit dependency-free invariant, adds model weight to the bundle,
   and raises licensing questions.

## Decision

Ship option 1 as an "Enhance" path in Resize (2×/4× presets applying
Lanczos + sharpen), with UI wording that says *sharper*, not *magic*.
Revisit option 2 when macOS exposes a stable system upscaler API that works
across the fleet. Option 3 is rejected for ToolboxKit as long as the
dependency-free invariant holds; if demand justifies it later, it belongs in a
separate target so the core stays clean.

## Consequences

Users get a real, honest improvement now; nobody is promised reconstruction of
faces and licence plates that this technique cannot deliver.
