import assert from "node:assert/strict";
import test from "node:test";
import {
  parseCapabilityRegistry,
  SharedCapabilityRegistry,
} from "../src/registry/capabilities.ts";
import {
  ToolWorkspaceRegistry,
  UtilityRegistry,
} from "../src/registry/index.ts";

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
      outputSuffix: shared.outputSuffix,
      manyToOne: shared.manyToOne,
    });
    assert.equal(
      utility.status,
      shared.nativeAvailability === "unavailable" ? "unavailable" : "implemented",
    );
  }

  assert.deepEqual(capabilitiesByCommand.get("remove_password")?.acceptedExtensions, [
    ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx",
  ]);
  assert.deepEqual(capabilitiesByCommand.get("protect_pdf")?.acceptedExtensions, [".pdf"]);
  assert.equal(capabilitiesByCommand.get("protect_pdf")?.nativeAvailability, "available");
  assert.deepEqual(capabilitiesByCommand.get("protect_office")?.acceptedExtensions, [
    ".docx", ".xlsx",
  ]);
  assert.equal(capabilitiesByCommand.get("protect_office")?.nativeAvailability, "available");
  assert.deepEqual(capabilitiesByCommand.get("images_to_pdf")?.acceptedExtensions, [
    ".png", ".jpg", ".jpeg", ".webp", ".heic",
  ]);
  assert.equal(capabilitiesByCommand.get("pdf_to_text")?.supportsPageSelection, true);
  assert.equal(capabilitiesByCommand.get("pdf_to_text")?.supportsPreview, false);
  assert.equal(capabilitiesByCommand.get("pdf_to_images")?.supportsPreview, false);
  assert.equal(capabilitiesByCommand.get("extract_pdf_pages")?.supportsPreview, false);
  assert.equal(capabilitiesByCommand.get("create_gif")?.inputCardinality, "ordered");
  assert.equal(capabilitiesByCommand.get("create_gif")?.supportsPreview, true);
  assert.equal(capabilitiesByCommand.get("extract_gif_frames")?.supportsPreview, false);
  assert.equal(capabilitiesByCommand.get("ocr_pdf")?.nativeAvailability, "unavailable");
  assert.equal(capabilitiesByCommand.get("blur_faces")?.nativeAvailability, "unavailable");
  assert.equal(capabilitiesByCommand.get("remove_image_background")?.nativeAvailability, "unavailable");
});

test("every tool declares one output suffix and many-to-one tools use the save step", () => {
  for (const capability of SharedCapabilityRegistry) {
    assert.match(capability.outputSuffix, /^-[a-z0-9]+(-[a-z0-9]+)*$/, capability.command);
    assert.equal(
      capability.manyToOne,
      capability.inputCardinality === "ordered",
      `${capability.command} many-to-one must match its ordered inputs`,
    );
  }
  assert.deepEqual(
    SharedCapabilityRegistry.filter(({ manyToOne }) => manyToOne)
      .map(({ command }) => command)
      .sort(),
    ["create_gif", "images_to_pdf", "merge_pdfs", "process_tiff_pages"],
  );
  const suffixes = new Map(
    SharedCapabilityRegistry.map(({ command, outputSuffix }) => [command, outputSuffix]),
  );
  assert.equal(suffixes.get("merge_pdfs"), "-merged");
  assert.equal(suffixes.get("protect_pdf"), "-protected");
  assert.equal(suffixes.get("pdf_to_images"), "-images");
});

test("workspace membership covers every registered utility exactly once", () => {
  const workspaceToolIds = ToolWorkspaceRegistry.flatMap((workspace) => workspace.toolIds);

  assert.equal(workspaceToolIds.length, UtilityRegistry.length);
  assert.equal(new Set(workspaceToolIds).size, workspaceToolIds.length);
  assert.deepEqual(
    [...workspaceToolIds].sort(),
    UtilityRegistry.map(({ id }) => id).sort(),
  );
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
  const { outputSuffix: _suffix, ...missingSuffix } = valid;
  assert.throws(() => parseCapabilityRegistry([missingSuffix]), /only the contract fields/);
  const { manyToOne: _manyToOne, ...missingManyToOne } = valid;
  assert.throws(() => parseCapabilityRegistry([missingManyToOne]), /only the contract fields/);
  assert.throws(
    () => parseCapabilityRegistry([{ ...valid, outputSuffixes: ["-a"] }]),
    /only the contract fields/,
  );
  assert.throws(
    () => parseCapabilityRegistry([{ ...valid, outputSuffix: ["-a", "-b"] }]),
    /exactly one valid output suffix/,
  );
  assert.throws(
    () => parseCapabilityRegistry([{ ...valid, outputSuffix: "unlocked" }]),
    /exactly one valid output suffix/,
  );
  assert.throws(
    () => parseCapabilityRegistry([{ ...valid, manyToOne: "yes" }]),
    /invalid many-to-one flag/,
  );
});
