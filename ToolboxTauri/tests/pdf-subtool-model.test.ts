import assert from "node:assert/strict";
import test from "node:test";
import { sceneFromDocument, type PdfScene, type SceneObject } from "../src/features/pdf-editor/scene";
import { exportSuffix, planSceneExport, subTools } from "../src/features/pdf-editor/subtools";

const baseline = sceneFromDocument({
  path: "/local/source.pdf",
  pages: [0, 1].map((index) => ({ index, width: 612, height: 792 })),
});
const object = (kind: SceneObject["kind"]): SceneObject => ({
  id: `${kind}-1`, kind, rect: { x: 10, y: 10, width: 100, height: 40 }, text: "Text", fontSize: 18, color: "#000000", opacity: 1, strokes: [],
});
const withObjects = (scene: PdfScene, pageIndex: number, ...kinds: SceneObject["kind"][]): PdfScene => ({
  pages: scene.pages.map((page, index) => index === pageIndex ? { ...page, objects: [...page.objects, ...kinds.map(object)] } : page),
});
const plan = (scene: PdfScene) => planSceneExport({ paths: ["/local/source.pdf"], scene, baseline, outputLocation: "alongsideInput" });

test("every Edit PDF sub-tool is declared once with a group and a suffix", () => {
  assert.deepEqual(subTools.map(({ id, group, suffix }) => [id, group, suffix]), [
    ["text", "overlay", "-edited"],
    ["highlight", "overlay", "-edited"],
    ["shape", "overlay", "-edited"],
    ["signature", "overlay", "-edited"],
    ["watermark", "overlay", "-edited"],
    ["crop", "document", "-edited"],
    ["organize", "document", "-edited"],
  ]);
});

test("export request carries the scene unchanged in the native request shape", () => {
  const scene = withObjects(baseline, 0, "watermark");
  const { request } = plan(scene);
  assert.deepEqual(Object.keys(request), ["paths", "scene", "outputLocation"]);
  assert.equal(request.scene, scene);
  assert.equal(JSON.stringify(request), JSON.stringify({ paths: ["/local/source.pdf"], scene, outputLocation: "alongsideInput" }));
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

test("an unchanged scene has no contributors and no suffix", () => {
  assert.deepEqual(plan(baseline), {
    request: { paths: ["/local/source.pdf"], scene: baseline, outputLocation: "alongsideInput" },
    contributors: [],
    suffix: null,
  });
  assert.deepEqual(plan(sceneFromDocument({ path: "/local/source.pdf", pages: [0, 1].map((index) => ({ index, width: 612, height: 792 })) })).contributors, []);
});
