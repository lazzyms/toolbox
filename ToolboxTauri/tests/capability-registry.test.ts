import assert from "node:assert/strict";
import test from "node:test";
import {
  parseCapabilityRegistry,
  SharedCapabilityRegistry,
} from "../src/registry/capabilities.ts";
import { UtilityRegistry } from "../src/registry/index.ts";

test("UI capabilities are the shared native contract facts", () => {
  const capabilitiesByCommand = new Map(
    SharedCapabilityRegistry.map((capability) => [capability.command, capability]),
  );

  assert.equal(capabilitiesByCommand.size, SharedCapabilityRegistry.length);
  for (const utility of UtilityRegistry) {
    const shared = capabilitiesByCommand.get(utility.command);
    assert.ok(shared, `Missing shared capability for ${utility.command}`);
    assert.deepEqual(utility.capability, {
      acceptedExtensions: shared.acceptedExtensions,
      inputCardinality: shared.inputCardinality,
      supportsPageSelection: shared.supportsPageSelection,
      supportsPreview: shared.supportsPreview,
      nativeAvailability: shared.nativeAvailability,
    });
    assert.equal(
      utility.status,
      shared.nativeAvailability === "unavailable" ? "unavailable" : "implemented",
    );
  }

  assert.deepEqual(capabilitiesByCommand.get("remove_password")?.acceptedExtensions, [
    ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx",
  ]);
  assert.deepEqual(capabilitiesByCommand.get("images_to_pdf")?.acceptedExtensions, [
    ".png", ".jpg", ".jpeg", ".webp", ".heic",
  ]);
  assert.equal(capabilitiesByCommand.get("pdf_to_text")?.supportsPageSelection, true);
  assert.equal(capabilitiesByCommand.get("pdf_to_text")?.supportsPreview, false);
  assert.equal(capabilitiesByCommand.get("extract_pdf_pages")?.supportsPreview, false);
  assert.equal(capabilitiesByCommand.get("create_gif")?.inputCardinality, "ordered");
  assert.equal(capabilitiesByCommand.get("create_gif")?.supportsPreview, true);
  assert.equal(capabilitiesByCommand.get("extract_gif_frames")?.supportsPreview, false);
  assert.equal(capabilitiesByCommand.get("ocr_pdf")?.nativeAvailability, "unavailable");
  assert.equal(capabilitiesByCommand.get("blur_faces")?.nativeAvailability, "unavailable");
  assert.equal(capabilitiesByCommand.get("remove_image_background")?.nativeAvailability, "unavailable");
});

test("shared capability parsing rejects drift-prone shapes", () => {
  const valid = SharedCapabilityRegistry[0];
  assert.throws(
    () => parseCapabilityRegistry([{ ...valid, supportsPreviews: true }]),
    /only the contract fields/,
  );
  assert.throws(
    () => parseCapabilityRegistry([valid, valid]),
    /Duplicate shared capability command/,
  );
});
