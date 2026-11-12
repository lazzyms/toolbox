import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";
import { saveStepAppliesTo, saveStepCommands, saveStepFor } from "../src/registry/saveSteps.ts";
import {
  capabilityFor,
  parseCapabilityRegistry,
  SharedCapabilityRegistry,
} from "../src/registry/capabilities.ts";
import { convertImagesRequest, convertTargets, defaultTiffCompression, tiffCompressions } from "../src/registry/convertTargets.ts";
import {
  ToolWorkspaceRegistry,
  UtilityRegistry,
  isPdfSceneTool,
  toolsForWorkspace,
  workspaceEntries,
  workspaceForTool,
} from "../src/registry/index.ts";
import { isMediaToolId, mediaTools, type MediaOptions, type MediaToolId } from "../src/features/media/index.ts";
import { estimateGifBytes, isLargeGif, LARGE_GIF_BYTES, playedDelayMs } from "../src/features/media/gifTiming.ts";
import { classifyIntake, entryStatuses, intakePolicyFor } from "../src/components/intake.ts";

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
      skippedExtensions: shared.skippedExtensions,
      inputCardinality: shared.inputCardinality,
      supportsPageSelection: shared.supportsPageSelection,
      supportsPreview: shared.supportsPreview,
      nativeAvailability: shared.nativeAvailability,
      outputSuffix: shared.outputSuffix,
      manyToOne: shared.manyToOne,
      ...(shared.formatFamilies ? { formatFamilies: shared.formatFamilies } : {}),
      ...(shared.outputFormats ? { outputFormats: shared.outputFormats } : {}),
    });
    assert.equal(
      utility.status,
      shared.nativeAvailability === "unavailable" ? "unavailable" : "implemented",
    );
  }

  assert.deepEqual(capabilitiesByCommand.get("remove_password")?.acceptedExtensions, [
    ".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx",
  ]);
  assert.equal(capabilitiesByCommand.get("remove_password")?.outputSuffix, "-unlocked");
  assert.equal(capabilitiesByCommand.get("remove_password")?.inputCardinality, "multiple");
  assert.equal(capabilitiesByCommand.get("convert_images")?.inputCardinality, "multiple");
  assert.deepEqual(capabilitiesByCommand.get("convert_images")?.acceptedExtensions, [
    ".png", ".jpg", ".jpeg", ".webp", ".heic", ".heif", ".tif", ".tiff",
  ]);
  assert.equal(capabilitiesByCommand.get("extract_gif_frames")?.inputCardinality, "multiple");
  assert.equal(capabilitiesByCommand.get("extract_gif_frames")?.manyToOne, false);
  assert.equal(UtilityRegistry.find(({ id }) => id === "lock-unlock")?.command, "remove_password");
  assert.equal(capabilitiesByCommand.get("split_pdf")?.inputCardinality, "multiple");
  assert.equal(capabilitiesByCommand.get("export_image_edit_stack")?.inputCardinality, "multiple");
  assert.equal(capabilitiesByCommand.get("export_image_edit_stack")?.outputSuffix, "-edited");
  for (const retired of ["crop_images", "resize_images", "rotate_images", "adjust_image_tone", "watermark_images", "image_metadata"]) {
    assert.equal(capabilitiesByCommand.get(retired), undefined, `${retired} is retired: Edit Image exports through the stack`);
  }
  for (const id of ["crop", "resize", "rotate", "image-tone", "image-watermark", "image-metadata"]) {
    assert.equal(UtilityRegistry.find((tool) => tool.id === id)?.command, "export_image_edit_stack", `${id} opens its step in the shared edit stack`);
  }
  const crop = UtilityRegistry.find(({ id }) => id === "crop");
  assert.equal(crop?.command, "export_image_edit_stack");
  assert.equal(crop?.capability.inputCardinality, "multiple", "Crop accepts a batch because it runs through the shared edit stack");
  assert.deepEqual(capabilitiesByCommand.get("protect_files")?.acceptedExtensions, [".pdf", ".doc", ".docx", ".xls", ".xlsx", ".ppt", ".pptx"]);
  assert.equal(capabilitiesByCommand.get("protect_files")?.nativeAvailability, "available");
  assert.equal(capabilitiesByCommand.get("protect_office"), undefined);
  assert.deepEqual(capabilitiesByCommand.get("images_to_pdf")?.acceptedExtensions, [
    ".png", ".jpg", ".jpeg", ".webp", ".heic", ".heif", ".tif", ".tiff",
  ]);
  assert.deepEqual(capabilitiesByCommand.get("compress_files")?.acceptedExtensions, [".pdf", ".jpg", ".jpeg", ".png", ".webp"]);
  assert.deepEqual(capabilitiesByCommand.get("compress_files")?.skippedExtensions, [".heic", ".heif"]);
  assert.ok(SharedCapabilityRegistry.filter(({ command }) => command !== "compress_files").every(({ skippedExtensions }) => skippedExtensions.length === 0));
  assert.equal(capabilitiesByCommand.get("compress_images"), undefined, "Compress Images is retired into compress_files");
  assert.deepEqual(UtilityRegistry.filter(({ command }) => command.startsWith("compress")).map(({ id }) => id), ["compress-files"]);
  assert.equal(capabilitiesByCommand.get("images_to_pdf")?.acceptedExtensions.includes(".tif"), true);
  assert.equal(capabilitiesByCommand.get("images_to_pdf")?.acceptedExtensions.includes(".tiff"), true);
  assert.equal(capabilitiesByCommand.get("images_to_pdf")?.acceptedExtensions.includes(".gif"), false);
  assert.equal(capabilitiesByCommand.get("extract_from_pdf")?.supportsPageSelection, true);
  assert.equal(capabilitiesByCommand.get("extract_from_pdf")?.supportsPreview, false);
  assert.equal(capabilitiesByCommand.get("extract_from_pdf")?.inputCardinality, "multiple");
  assert.equal(capabilitiesByCommand.get("pdf_to_text"), undefined);
  assert.equal(capabilitiesByCommand.get("extract_pdf_images"), undefined);
  assert.equal(UtilityRegistry.find(({ id }) => id === "pdf-extract")?.command, "extract_from_pdf");
  assert.equal(capabilitiesByCommand.get("pdf_to_images")?.supportsPreview, false);
  assert.deepEqual(capabilitiesByCommand.get("pdf_to_images")?.outputFormats, [".jpg", ".png", ".webp"]);
  assert.equal(UtilityRegistry.find(({ command }) => command === "pdf_to_images")?.blurb, "Render PDF pages to JPEG, PNG or WebP images at 72 to 300 dpi.");
  assert.equal(capabilitiesByCommand.get("extract_pdf_pages")?.supportsPreview, false);
  assert.equal(capabilitiesByCommand.get("create_gif")?.inputCardinality, "ordered");
  assert.equal(capabilitiesByCommand.get("create_gif")?.supportsPreview, true);
  assert.equal(capabilitiesByCommand.get("extract_gif_frames")?.supportsPreview, false);
  assert.deepEqual(capabilitiesByCommand.get("inspect_pdf_privacy"), {
    command: "inspect_pdf_privacy",
    acceptedExtensions: [".pdf"],
    inputCardinality: "multiple",
    supportsPageSelection: false,
    supportsPreview: false,
    nativeAvailability: "available",
    outputSuffix: "",
    manyToOne: false,
    skippedExtensions: [],
  });
  assert.equal(UtilityRegistry.find(({ id }) => id === "pdf-privacy")?.command, "inspect_pdf_privacy");
  assert.deepEqual(capabilitiesByCommand.get("edit_pdf_properties"), {
    command: "edit_pdf_properties",
    acceptedExtensions: [".pdf"],
    inputCardinality: "multiple",
    supportsPageSelection: false,
    supportsPreview: false,
    nativeAvailability: "available",
    outputSuffix: "-edited",
    manyToOne: false,
    skippedExtensions: [],
  });
  assert.equal(UtilityRegistry.find(({ id }) => id === "pdf-properties")?.command, "edit_pdf_properties");
  for (const command of ["generate_icon_set", "create_gif"]) {
    assert.deepEqual(capabilitiesByCommand.get(command)?.acceptedExtensions, [".png", ".jpg", ".jpeg", ".webp", ".heic", ".heif"], command);
  }
  assert.equal(capabilitiesByCommand.get("ocr_pdf")?.nativeAvailability, "available");
  assert.equal(capabilitiesByCommand.get("ocr_pdf")?.inputCardinality, "multiple");
  assert.equal(capabilitiesByCommand.get("ocr_pdf")?.outputSuffix, "-ocr");
  assert.deepEqual(capabilitiesByCommand.get("ocr_pdf")?.acceptedExtensions, [".pdf", ".png", ".jpg", ".jpeg", ".webp", ".heic", ".tif", ".tiff"]);
  assert.equal(capabilitiesByCommand.get("blur_faces")?.nativeAvailability, "unavailable");
  assert.equal(capabilitiesByCommand.get("remove_image_background")?.nativeAvailability, "unavailable");
});

test("Convert Image Format writes TIFF, and only a TIFF target carries a compression", () => {
  assert.deepEqual(convertTargets.map(({ format }) => format), ["jpg", "png", "webp", "heic", "tiff"]);
  assert.equal(convertTargets.find(({ format }) => format === "tiff")?.hint, "Lossless. Transparency is kept.");
  assert.deepEqual(tiffCompressions.map(({ value }) => value), ["lzw", "none", "deflate"]);
  assert.equal(defaultTiffCompression, "lzw");
  const location = "alongsideInput" as const;
  assert.deepEqual(convertImagesRequest(["/a.png"], "tiff", "deflate", location), { paths: ["/a.png"], format: "tiff", tiffCompression: "deflate", outputLocation: location });
  for (const { format } of convertTargets.filter(({ format }) => format !== "tiff")) {
    assert.equal("tiffCompression" in convertImagesRequest(["/a.png"], format, "lzw", location), false, `${format} sends no compression`);
  }
});

test("every tool declares one output suffix and many-to-one tools take ordered inputs", () => {
  // Read-only tools write no file, so they declare an empty suffix.
  const readOnly = new Set(["inspect_pdf_privacy"]);
  for (const capability of SharedCapabilityRegistry) {
    if (readOnly.has(capability.command) || capability.formatFamilies) assert.equal(capability.outputSuffix, "", capability.command);
    else assert.match(capability.outputSuffix, /^-[a-z0-9]+(-[a-z0-9]+)*$/, capability.command);
    assert.ok(
      !capability.manyToOne || capability.inputCardinality === "ordered",
      `${capability.command} many-to-one must take ordered inputs`,
    );
  }
  assert.deepEqual(
    SharedCapabilityRegistry.filter(({ manyToOne }) => manyToOne)
      .map(({ command }) => command)
      .sort(),
    ["compare_pdfs", "create_gif", "images_to_pdf", "merge_pdfs", "process_tiff_pages"],
  );
  const suffixes = new Map(
    SharedCapabilityRegistry.map(({ command, outputSuffix }) => [command, outputSuffix]),
  );
  assert.equal(suffixes.get("merge_pdfs"), "-merged");
  assert.equal(suffixes.get("bates_number_pdfs"), "-bates");
  assert.equal(SharedCapabilityRegistry.find(({ command }) => command === "bates_number_pdfs")?.inputCardinality, "ordered");
  assert.equal(suffixes.get("protect_files"), "-protected");
  assert.equal(suffixes.get("pdf_to_images"), "-page");
  assert.equal(suffixes.get("convert_to_pdf"), "");
  assert.equal(suffixes.get("split_pdf"), "-page");
});

test("every many-to-one tool opens the save step and sends the chosen path", () => {
  const requestSources = ["views", "features"].flatMap((folder) =>
    readdirSync(new URL(`../src/${folder}/`, import.meta.url), { recursive: true, encoding: "utf8" })
      .filter((name) => /\.tsx?$/.test(name))
      .map((name) => readFileSync(new URL(`../src/${folder}/${name}`, import.meta.url), "utf8")),
  );
  const requestFor = (command: string) => {
    for (const source of requestSources) {
      // A view invokes the command directly; a Media unit returns it next to its request.
      const start = [`invoke<ToolResult>("${command}"`, `command: "${command}",`]
        .map((anchor) => source.indexOf(anchor))
        .find((index) => index !== -1);
      if (start === undefined) continue;
      const open = source.indexOf("request: {", start) + "request: ".length;
      let depth = 0;
      for (let index = open; index < source.length; index += 1) {
        if (source[index] === "{") depth += 1;
        if (source[index] === "}") depth -= 1;
        if (depth === 0) return source.slice(open, index + 1);
      }
    }
    return null;
  };

  // Media tools build their requests in src/features/media, so ask them directly, under the options in which
  // the tool is many-to-one.
  const manyToOneMediaOptions: Partial<MediaOptions> = {
    "tiff-pages": { ...mediaTools["tiff-pages"].initialOptions, mode: "combine" },
  };
  const mediaRequestFor = <Id extends MediaToolId>(id: Id) => {
    const tool = mediaTools[id];
    const options = (manyToOneMediaOptions[id] ?? tool.initialOptions) as MediaOptions[Id];
    return tool.buildRequest(options, { paths: ["/a", "/b"], keys: ["a", "b"], outputLocation: "alongsideInput", outputPath: "/chosen" }).request;
  };

  for (const utility of UtilityRegistry) {
    const step = saveStepFor(utility);
    if (!utility.capability.manyToOne) {
      assert.equal(step, null, `${utility.command} makes one file per input and must not open the save step`);
      continue;
    }
    assert.ok(step, `${utility.command} is many-to-one and must use the save step`);
    assert.ok(saveStepAppliesTo(step, 2), `${utility.command} must save a combine through the step`);
    assert.match(step.runLabel, /…$/, `${utility.command} opens a dialog, so its run label ends with an ellipsis`);
    if (isMediaToolId(utility.id)) {
      assert.equal(mediaRequestFor(utility.id).outputPath, "/chosen", `${utility.command} must send the chosen path`);
    } else {
      assert.match(requestFor(utility.command) ?? "", /\boutputPath\b/, `${utility.command} must send the chosen path`);
    }
  }
  assert.deepEqual(
    saveStepCommands().sort(),
    SharedCapabilityRegistry.filter(({ manyToOne }) => manyToOne).map(({ command }) => command).sort(),
  );
  assert.throws(() => saveStepFor({ command: "unknown_combiner", capability: { manyToOne: true } }), /has no save step/);
});

test("TIFF Pages names its mode in every request and is many-to-one only in Combine mode", () => {
  const utility = UtilityRegistry.find(({ command }) => command === "process_tiff_pages")!;
  const tool = mediaTools["tiff-pages"];
  // The registered capability describes Combine; Split is chosen explicitly and runs per file.
  assert.equal(utility.capability.manyToOne, true);
  assert.equal(tool.initialOptions.mode, "split");
  const input = { paths: ["/scans/a.tiff"], keys: ["a"], outputLocation: "alongsideInput" as const, outputPath: "/chosen.tiff" };

  const split = { ...tool.initialOptions, mode: "split" as const };
  const splitCapability = tool.capability!(split, utility.capability);
  assert.equal(splitCapability.manyToOne, false);
  assert.equal(splitCapability.inputCardinality, "multiple", "a rejected file in Split fails alone");
  assert.equal(saveStepFor({ command: utility.command, capability: splitCapability }), null, "Split never opens the save step");
  assert.deepEqual(tool.buildRequest(split, { ...input, paths: ["/scans/a.tiff", "/scans/b.tiff"], keys: ["a", "b"] }).request, {
    mode: "split", paths: ["/scans/a.tiff", "/scans/b.tiff"], outputLocation: "alongsideInput", compression: "lzw",
  });
  assert.equal(tool.initialOptions.compression, "lzw", "LZW is the default output compression");
  assert.equal(tool.buildRequest({ ...split, compression: "deflate" }, input).request.compression, "deflate");

  const combine = { ...tool.initialOptions, mode: "combine" as const };
  assert.equal(tool.capability!(combine, utility.capability), utility.capability);
  assert.ok(saveStepAppliesTo(saveStepFor(utility), 1), "one file in Combine mode still makes one TIFF through the save step");
  assert.deepEqual(tool.buildRequest(combine, input).request, {
    mode: "combine", paths: ["/scans/a.tiff"], pages: [], outputLocation: "alongsideInput", outputPath: "/chosen.tiff", compression: "lzw",
  });
  assert.equal(tool.buildRequest({ ...combine, compression: "none" }, input).request.compression, "none");

  // Pages follow the person's order across files; left-out pages are not sent, and new pages follow in file order.
  const preview = { width: 1, height: 1, dataUrl: "" };
  const arranged = {
    ...combine,
    sources: { a: { kind: "ready" as const, previews: [preview, preview] }, b: { kind: "ready" as const, previews: [preview] } },
    order: ["b\u00000", "a\u00001"],
    leftOut: ["a\u00001"],
  };
  const both = { ...input, paths: ["/scans/a.tiff", "/scans/b.tiff"], keys: ["a", "b"] };
  assert.deepEqual(tool.buildRequest(arranged, both).request.pages, [
    { path: "/scans/b.tiff", page: 0 },
    { path: "/scans/a.tiff", page: 0 },
  ]);
  assert.equal(tool.canRun(arranged, both.paths, both.keys), true);
  assert.equal(tool.runLabel?.(arranged, both.paths, both.keys), "Combine 2 pages…");
  assert.equal(tool.canRun({ ...arranged, sources: { ...arranged.sources, b: { kind: "loading" } } }, both.paths, both.keys), false, "Combine waits for every file's pages");
  assert.equal(tool.canRun({ ...arranged, leftOut: ["a\u00000", "a\u00001", "b\u00000"] }, both.paths, both.keys), false, "Combine needs at least one page");

  // Fax (Group 3 and 4) and JPEG-compressed TIFF scans are decoded in-process, so both modes take them without a message.
  const scans = ["/scans/fax-g3.tif", "/scans/fax-g3-2d.TIFF", "/scans/fax-g4.tiff", "/scans/scan-jpeg.tif"];
  for (const options of [split, combine]) {
    const active = { ...utility, capability: tool.capability!(options, utility.capability) };
    const intake = classifyIntake([], scans.map((path) => ({ path, kind: "file" as const })), intakePolicyFor(utility.title, [active], active));
    assert.deepEqual(intake, { added: scans.map((path) => ({ path, kind: "file" })), notAdded: [] }, `${options.mode} adds every fax and JPEG scan`);
    const statuses = entryStatuses(scans.map((path, index) => ({ key: String(index), path })), active);
    assert.deepEqual(statuses.map(({ flag }) => flag), scans.map(() => null), `${options.mode} flags none of them`);
  }
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

test("Compress Files is one tool that both PDFs and images reach", () => {
  const compress = ToolWorkspaceRegistry.find(({ id }) => id === "compress-files")!;
  assert.deepEqual(compress.toolIds, ["compress-files"]);
  assert.deepEqual(workspaceEntries(compress, toolsForWorkspace(compress)).map(({ title }) => title), ["Compress"]);
  for (const workspace of ToolWorkspaceRegistry) {
    assert.ok(!workspace.editor?.toolIds.some((id) => id.startsWith("compress")), `${workspace.title} editor must not offer a compress step`);
  }
});

test("workspace entries group editor tools into one entry and list labelled groups after the main tools", () => {
  const entryIds = (workspaceId: string, options?: { groupEditor?: boolean }) => {
    const workspace = ToolWorkspaceRegistry.find(({ id }) => id === workspaceId)!;
    return workspaceEntries(workspace, toolsForWorkspace(workspace), options)
      .map((entry) => `${entry.group ? `${entry.group.toLowerCase()}:` : ""}${entry.tools.map(({ id }) => id).join("+")}`);
  };

  assert.deepEqual(entryIds("pdf-editor"), [
    "pdf-edit+pdf-watermark+pdf-sign+pdf-remove-pages",
    "pdf-organize", "pdf-merge", "pdf-split", "pdf-crop", "pdf-page-numbers", "pdf-redact",
    "details:pdf-privacy", "details:pdf-properties",
    "more:pdf-rotate", "more:pdf-repair", "more:pdf-compare", "more:pdf-extract-pages", "more:pdf-bates",
  ]);
  assert.deepEqual(entryIds("image-editor"), ["resize+crop+rotate+image-tone+image-watermark+image-metadata", "heic-convert"]);
  assert.deepEqual(entryIds("image-editor", { groupEditor: false }), ["resize", "crop", "rotate", "image-tone", "image-watermark", "image-metadata", "heic-convert"]);

  const pdfEditor = ToolWorkspaceRegistry.find(({ id }) => id === "pdf-editor")!;
  const [editEntry] = workspaceEntries(pdfEditor, toolsForWorkspace(pdfEditor).filter(({ id }) => id !== "pdf-edit"));
  assert.equal(editEntry.title, "Edit PDF");
  assert.equal(editEntry.tools[0].id, "pdf-watermark", "a hidden tool leaves the group led by the next one");
});

test("Crop and Resize PDF is a PDF Editor batch tool that takes many PDFs", () => {
  const crop = UtilityRegistry.find(({ id }) => id === "pdf-crop")!;
  assert.equal(crop.title, "Crop and Resize PDF");
  assert.equal(crop.command, "crop_pdf");
  assert.equal(workspaceForTool("pdf-crop")?.id, "pdf-editor");
  assert.equal(isPdfSceneTool("pdf-crop"), false, "the rail entry opens the batch view, not the page editor");
  assert.equal(crop.capability.inputCardinality, "multiple");
  assert.deepEqual(crop.capability.acceptedExtensions, [".pdf"]);
  const resize = capabilityFor("resize_pdf");
  assert.equal(resize.outputSuffix, "-resized");
  assert.deepEqual({ ...resize, outputSuffix: "-cropped" }, capabilityFor("crop_pdf"), "Resize shares every input rule with Crop");
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
  const { skippedExtensions: _skipped, ...withoutSkips } = valid;
  assert.deepEqual(parseCapabilityRegistry([withoutSkips])[0].skippedExtensions, []);
  for (const skippedExtensions of [[".HEIC"], ["heic"], valid.acceptedExtensions.slice(0, 1), ".heic"]) {
    assert.throws(() => parseCapabilityRegistry([{ ...valid, skippedExtensions }]), /invalid accepted extensions/);
  }
});

test("shared capability parsing reads optional output formats, including WebP for PDF to Images", () => {
  const valid = SharedCapabilityRegistry.find(({ command }) => command === "extract_from_pdf")!;
  assert.equal(parseCapabilityRegistry([valid])[0].outputFormats, undefined);
  assert.deepEqual(
    parseCapabilityRegistry([{ ...valid, outputFormats: [".jpg", ".png", ".webp"] }])[0].outputFormats,
    [".jpg", ".png", ".webp"],
  );
  for (const outputFormats of [[], ["webp"], [".WebP"], [".webp", ".webp"], ".webp", [".we.bp"]]) {
    assert.throws(
      () => parseCapabilityRegistry([{ ...valid, outputFormats }]),
      /invalid output formats/,
      JSON.stringify(outputFormats),
    );
  }
});

test("shared capability samples parse the same as the native contract parser", () => {
  // The Rust contracts test reads the same samples, so both parsers agree on every row.
  const samples = JSON.parse(
    readFileSync(new URL("../shared/tool-capability-samples.json", import.meta.url), "utf8"),
  ) as {
    accepted: Record<string, unknown>[];
    rejected: { name: string; capability: Record<string, unknown> }[];
  };

  for (const row of samples.accepted) {
    const [parsed] = parseCapabilityRegistry([row]);
    assert.equal(parsed.inputCardinality, row.inputCardinality, String(row.command));
    assert.deepEqual(parsed.acceptedExtensions, row.acceptedExtensions, String(row.command));
  }
  for (const { name, capability } of samples.rejected) {
    assert.throws(() => parseCapabilityRegistry([capability]), /accepted extension/, name);
  }

  const creation = samples.accepted.find(({ command }) => command === "sample_create_document")!;
  assert.equal(parseCapabilityRegistry([creation])[0].inputCardinality, "none");
  assert.throws(
    () => parseCapabilityRegistry([{ ...creation, acceptedExtensions: [".pdf"] }]),
    /takes no input files, so it cannot declare accepted extensions/,
  );
  assert.throws(
    () => parseCapabilityRegistry([{ ...creation, inputCardinality: "zero" }]),
    /invalid input cardinality/,
  );
});

const familySamples = JSON.parse(
  readFileSync(new URL("../shared/capability-family-samples.json", import.meta.url), "utf8"),
);

test("format families derive accepted extensions and per-family flags like the native parser", () => {
  const { row, acceptedExtensions, families } = familySamples.valid;
  const [capability] = parseCapabilityRegistry([row]);

  assert.deepEqual(capability.acceptedExtensions, acceptedExtensions);
  assert.deepEqual(
    capability.formatFamilies?.map(({ id, nativeAvailability, basicFidelity }) => ({ id, nativeAvailability, basicFidelity })),
    families,
  );
  assert.deepEqual(SharedCapabilityRegistry.filter((shared) => shared.formatFamilies).map(({ command }) => command), ["convert_to_pdf"]);
  const convert = SharedCapabilityRegistry.find(({ command }) => command === "convert_to_pdf")!;
  assert.deepEqual(convert.acceptedExtensions, convert.formatFamilies!.flatMap(({ extensions }) => extensions));
  assert.deepEqual(
    convert.formatFamilies!.filter(({ nativeAvailability }) => nativeAvailability === "available").map(({ id }) => id),
    ["plain-text", "pdf", "word", "rich-text", "spreadsheets", "presentations", "markdown", "web-pages", "tables", "ebooks", "legacy-office", "mobi"],
  );
});

test("format families reject shared extensions, empty families and mixed extension shapes", () => {
  const { formatFamilies: _families, ...base } = familySamples.valid.row;
  for (const [name, shape] of Object.entries(familySamples.invalid)) {
    assert.throws(() => parseCapabilityRegistry([{ ...base, ...(shape as object) }]), Error, `${name} must be rejected`);
  }
  assert.throws(() => parseCapabilityRegistry([base]), /either accepted extensions or format families/);
});

test("Create GIF preview plays each delay as the GIF stores it, in hundredths of a second", () => {
  assert.equal(playedDelayMs(44), 40);
  assert.equal(playedDelayMs(45), 50);
  assert.equal(playedDelayMs(100), 100);
  assert.equal(playedDelayMs(1), 10);
  assert.equal(playedDelayMs(4), 10);
  assert.equal(playedDelayMs(60_000), 60_000);
});

test("Create GIF warns above 10 MB, estimated as one byte per canvas pixel per frame", () => {
  assert.equal(LARGE_GIF_BYTES, 10_000_000);
  assert.equal(estimateGifBytes({ width: 480, height: 360 }, 12), 2_073_600);
  assert.equal(isLargeGif(10_000_000), false);
  assert.equal(isLargeGif(10_000_001), true);
  assert.equal(isLargeGif(estimateGifBytes({ width: 2000, height: 1000 }, 5)), false);
  assert.equal(isLargeGif(estimateGifBytes({ width: 2000, height: 1000 }, 6)), true);
});
