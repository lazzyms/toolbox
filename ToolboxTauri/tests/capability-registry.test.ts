import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { UtilityRegistry } from "../src/registry/index.ts";

interface SharedCapability {
  command: string;
  acceptedExtensions: string[];
  inputCardinality: "single" | "multiple" | "ordered";
  supportsPageSelection: boolean;
  supportsPreview: boolean;
  nativeAvailability: "available" | "unavailable";
}

const sharedCapabilities = JSON.parse(
  readFileSync(new URL("../shared/tool-capabilities.json", import.meta.url), "utf8"),
) as SharedCapability[];

test("UI capabilities are the shared native contract facts", () => {
  const capabilitiesByCommand = new Map(
    sharedCapabilities.map((capability) => [capability.command, capability]),
  );

  assert.equal(capabilitiesByCommand.size, sharedCapabilities.length);
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
  }

  assert.deepEqual(capabilitiesByCommand.get("remove_password")?.acceptedExtensions, [
    ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx",
  ]);
  assert.deepEqual(capabilitiesByCommand.get("images_to_pdf")?.acceptedExtensions, [
    ".png", ".jpg", ".jpeg", ".webp", ".heic", ".tif", ".tiff",
  ]);
  assert.equal(capabilitiesByCommand.get("pdf_to_text")?.supportsPageSelection, true);
  assert.equal(capabilitiesByCommand.get("create_gif")?.inputCardinality, "ordered");
  assert.equal(capabilitiesByCommand.get("ocr_pdf")?.nativeAvailability, "unavailable");
  assert.equal(capabilitiesByCommand.get("blur_faces")?.nativeAvailability, "unavailable");
  assert.equal(capabilitiesByCommand.get("remove_image_background")?.nativeAvailability, "unavailable");
});
