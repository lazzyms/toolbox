import assert from "node:assert/strict";
import test from "node:test";
import { sceneFromDocument, type PdfScene, type SceneObject } from "../src/features/pdf-editor/scene";
import { textOverflowWords } from "../src/features/pdf-editor/scene";
import { exportSuffix, planSceneExport, sceneSections, subTools } from "../src/features/pdf-editor/subtools";

const baseline = sceneFromDocument({
  path: "/local/source.pdf",
  pages: [0, 1].map((index) => ({ index, width: 612, height: 792 })),
});
const object = (kind: SceneObject["kind"]): SceneObject => ({
  id: `${kind}-1`, kind, rect: { x: 10, y: 10, width: 100, height: 40 }, text: "Text", fontSize: 18, color: "#000000", opacity: 1,
});
const withObjects = (scene: PdfScene, pageIndex: number, ...kinds: SceneObject["kind"][]): PdfScene => ({
  pages: scene.pages.map((page, index) => index === pageIndex ? { ...page, objects: [...page.objects, ...kinds.map(object)] } : page),
});
const plan = (scene: PdfScene) => planSceneExport({ paths: ["/local/source.pdf"], scene, baseline, outputLocation: "alongsideInput" });

test("every Edit PDF sub-tool is declared once with a group and a suffix", () => {
  assert.deepEqual(subTools.map(({ id, group, suffix }) => [id, group, suffix]), [
    ["form", "document", "-filled"],
    ["edit-text", "document", "-edited"],
    ["text", "overlay", "-edited"],
    ["highlight", "overlay", "-edited"],
    ["shape", "overlay", "-edited"],
    ["signature", "overlay", "-edited"],
    ["watermark", "overlay", "-edited"],
    ["pageNumbers", "overlay", "-edited"],
    ["crop", "document", "-edited"],
    ["organize", "document", "-edited"],
    ["redact", "document", "-redacted"],
  ]);
});

test("export request carries the scene unchanged in the native request shape", () => {
  const scene = withObjects(baseline, 0, "watermark");
  const { request } = plan(scene);
  assert.deepEqual(Object.keys(request), ["paths", "scene", "outputLocation"]);
  assert.deepEqual(request.scene, scene);
  assert.equal(JSON.stringify(request), JSON.stringify({ paths: ["/local/source.pdf"], scene, outputLocation: "alongsideInput" }));
});

test("Export As adds the chosen path after the destination and a plain export omits it", () => {
  const scene = withObjects(baseline, 0, "shape");
  const { request } = planSceneExport({ paths: ["/local/source.pdf"], scene, baseline, outputLocation: "alongsideInput", outputPath: "/local/contract final.pdf" });
  assert.deepEqual(Object.keys(request), ["paths", "scene", "outputLocation", "outputPath"]);
  assert.equal(request.outputPath, "/local/contract final.pdf");
  assert.equal("outputPath" in plan(scene).request, false);
});

test("chained export reports contributors in the fixed export order, not scene order", () => {
  const edited = withObjects(baseline, 0, "watermark", "signature", "text");
  const scene: PdfScene = {
    pages: [{ ...edited.pages[1], rotation: 90, crop: { x: 0, y: 0, width: 300, height: 300 } }, edited.pages[0]],
  };
  assert.deepEqual(plan(scene).contributors, ["text", "signature", "watermark", "crop", "organize"]);
});

test("contributor detection follows each sub-tool's own rule", () => {
  assert.deepEqual(plan(withObjects(baseline, 1, "highlight")).contributors, ["highlight"]);
  assert.deepEqual(plan(withObjects(baseline, 0, "shape", "shape")).contributors, ["shape"]);
  assert.deepEqual(plan({ pages: [{ ...baseline.pages[0], crop: { x: 1, y: 1, width: 10, height: 10 } }, baseline.pages[1]] }).contributors, ["crop"]);
  assert.deepEqual(plan({ pages: [...baseline.pages].reverse() }).contributors, ["organize"]);
  assert.deepEqual(plan({ pages: [baseline.pages[0]] }).contributors, ["organize"]);
  assert.deepEqual(plan({ pages: [{ ...baseline.pages[0], rotation: 180 }, baseline.pages[1]] }).contributors, ["organize"]);
});

test("a single contributor exports with its own suffix and several use -edited", () => {
  assert.equal(exportSuffix([{ suffix: "-redacted" }]), "-redacted");
  assert.equal(exportSuffix([{ suffix: "-filled" }, { suffix: "-redacted" }]), "-edited");
  assert.equal(plan(withObjects(baseline, 0, "signature")).suffix, "-edited");
  assert.equal(plan(withObjects(baseline, 0, "signature", "watermark")).suffix, "-edited");
});

test("form values join the request only when present and fill first", () => {
  const formValues = [{ name: "applicant.name", value: "Ada" }, { name: "agree", value: true }];
  const filled = planSceneExport({ paths: ["/local/source.pdf"], scene: baseline, baseline, outputLocation: "alongsideInput", formValues });
  assert.deepEqual(Object.keys(filled.request), ["paths", "scene", "formValues", "outputLocation"]);
  assert.deepEqual(filled.request.formValues, formValues);
  assert.deepEqual(filled.contributors, ["form"]);
  assert.equal(filled.suffix, "-filled");
  const chained = planSceneExport({ paths: ["/local/source.pdf"], scene: withObjects(baseline, 0, "text"), baseline, outputLocation: "alongsideInput", formValues });
  assert.deepEqual(chained.contributors, ["form", "text"]);
  assert.equal(chained.suffix, "-edited");
  const empty = planSceneExport({ paths: ["/local/source.pdf"], scene: baseline, baseline, outputLocation: "alongsideInput", formValues: [] });
  assert.equal("formValues" in empty.request, false);
  assert.equal(empty.suffix, null);
});

test("an unchanged scene has no contributors and no suffix", () => {
  assert.deepEqual(plan(baseline), {
    request: { paths: ["/local/source.pdf"], scene: baseline, outputLocation: "alongsideInput" },
    contributors: [],
    suffix: null,
  });
  assert.deepEqual(plan(sceneFromDocument({ path: "/local/source.pdf", pages: [0, 1].map((index) => ({ index, width: 612, height: 792 })) })).contributors, []);
});

test("redaction marks leave the scene and travel as the request's redactions section in output page order", () => {
  const mark = (id: string, x: number, color = "#000000") => ({ id, rect: { x, y: 20, width: 50, height: 10 }, color });
  const scene: PdfScene = { pages: [{ ...baseline.pages[1], redactions: [mark("b", 5, "#ffffff")] }, { ...baseline.pages[0], redactions: [mark("a", 1), mark("c", 9)] }] };
  const result = plan(scene);
  assert.deepEqual(result.contributors, ["organize", "redact"]);
  assert.equal(result.suffix, "-edited");
  assert.deepEqual(Object.keys(result.request), ["paths", "scene", "redactions", "outputLocation"]);
  assert.deepEqual(result.request.redactions, [
    { pageIndex: 0, rect: { x: 5, y: 20, width: 50, height: 10 }, color: "#ffffff" },
    { pageIndex: 1, rect: { x: 1, y: 20, width: 50, height: 10 }, color: "#000000" },
    { pageIndex: 1, rect: { x: 9, y: 20, width: 50, height: 10 }, color: "#000000" },
  ]);
  assert.ok(result.request.scene.pages.every((page) => !("redactions" in page)));
  assert.deepEqual(result.request.scene.pages.map((page) => page.id), ["source-1", "source-0"]);

  const only = plan({ pages: [{ ...baseline.pages[0], redactions: [mark("a", 1)] }, baseline.pages[1]] });
  assert.deepEqual(only.contributors, ["redact"]);
  assert.equal(only.suffix, "-redacted");
  assert.deepEqual(plan({ pages: [{ ...baseline.pages[0], redactions: [] }, baseline.pages[1]] }).contributors, []);
});

test("text edits travel as their own section beside the scene and contribute -edited", () => {
  const edit = { pageIndex: 1, run: 0, original: "Rent £1,450", replacement: "Rent £1,475" };
  const scene: PdfScene = { ...baseline, textEdits: [edit] };
  const result = plan(scene);
  assert.deepEqual(result.contributors, ["edit-text"]);
  assert.equal(result.suffix, "-edited");
  assert.equal(JSON.stringify(result.request), JSON.stringify({ paths: ["/local/source.pdf"], scene: { pages: baseline.pages }, textEdits: [edit], outputLocation: "alongsideInput" }));
  assert.deepEqual(sceneSections({ ...baseline, textEdits: [] }), { scene: { pages: baseline.pages } });
  assert.deepEqual(plan({ ...withObjects(baseline, 0, "text"), textEdits: [edit] }).contributors, ["edit-text", "text"]);
});

test("overflow counts the words that end past the original text length", () => {
  assert.equal(textOverflowWords("Rent £1,450", "Rent £1,475"), 0);
  assert.equal(textOverflowWords("Rent £1,450", "Rent £1,475 per calendar month"), 3);
  assert.equal(textOverflowWords("Total", "Totals"), 1);
  assert.equal(textOverflowWords("Long original line", "Short"), 0);
});
