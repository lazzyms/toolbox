import type { OutputLocation } from '../../contracts';
import type { PdfFlattenChoice, PdfFormValue as FormValue, PdfTextEdit } from './contracts';
import type { PdfScene, SceneObject, SceneRect } from './scene';

export type SubToolId = 'form' | SceneObject['kind'] | 'edit-text' | 'crop' | 'organize' | 'flatten' | 'redact';
export interface SubTool {
  id: SubToolId;
  label: string;
  group: 'overlay' | 'document';
  suffix: string;
  contributes: (scene: PdfScene, baseline: PdfScene, formValues: readonly FormValue[], flatten: PdfFlattenChoice | null) => boolean;
}
export interface RedactionMark { pageIndex: number; rect: SceneRect; color: string }
export interface SceneSections { scene: Pick<PdfScene, 'pages'>; formValues?: FormValue[]; textEdits?: PdfTextEdit[]; flatten?: PdfFlattenChoice; redactions?: RedactionMark[] }
export interface SceneExportRequest extends SceneSections { paths: string[]; outputLocation: OutputLocation; outputPath?: string }
export interface SceneExportPlan { request: SceneExportRequest; contributors: SubToolId[]; summary: ExportStep[]; suffix: string | null }
/** One row of the export summary: a stage of the native export and the sub-tools that feed it. */
export interface ExportStep { stage: ExportStageId; label: string; tools: string[] }
export type ExportStageId = 'fill-forms' | 'edit-text' | 'overlays' | 'flatten' | 'redact';

export const combinedSuffix = '-edited';

const overlay = (id: SceneObject['kind'], label: string): SubTool => ({
  id, label, group: 'overlay', suffix: '-edited',
  contributes: (scene) => scene.pages.some((page) => page.objects.some((object) => object.kind === id)),
});
const pageLayout = (scene: PdfScene) => JSON.stringify(scene.pages.map((page) => [page.id, page.rotation, Boolean(page.mirrored)]));

export const subTools: readonly SubTool[] = [
  { id: 'form', label: 'Fill form', group: 'document', suffix: '-filled',
    contributes: (_scene, _baseline, formValues) => formValues.length > 0 },
  { id: 'edit-text', label: 'Edit text', group: 'document', suffix: '-edited',
    contributes: (scene) => Boolean(scene.textEdits?.length) },
  overlay('text', 'Text'),
  overlay('highlight', 'Highlight'),
  overlay('shape', 'Shape'),
  overlay('signature', 'Signature'),
  overlay('watermark', 'Watermark'),
  overlay('pageNumbers', 'Page numbers'),
  overlay('headerFooter', 'Header and footer'),
  { id: 'crop', label: 'Crop', group: 'document', suffix: '-edited',
    contributes: (scene) => scene.pages.some((page) => page.crop !== null) },
  { id: 'organize', label: 'Organize pages', group: 'document', suffix: '-edited',
    contributes: (scene, baseline) => pageLayout(scene) !== pageLayout(baseline) },
  { id: 'flatten', label: 'Flatten', group: 'document', suffix: '-flattened',
    contributes: (_scene, _baseline, _formValues, flatten) => flatten !== null },
  { id: 'redact', label: 'Redact', group: 'document', suffix: '-redacted',
    contributes: (scene) => scene.pages.some((page) => page.redactions?.length) },
];

/** The native export's stages, in the fixed order it applies them. Each sub-tool feeds exactly one stage. */
export const exportStages: readonly { id: ExportStageId; label: string; tools: readonly SubToolId[] }[] = [
  { id: 'fill-forms', label: 'Fill form', tools: ['form'] },
  { id: 'edit-text', label: 'Edit text', tools: ['edit-text'] },
  { id: 'overlays', label: 'Add text, signatures and page marks', tools: ['text', 'highlight', 'shape', 'signature', 'watermark', 'pageNumbers', 'headerFooter', 'crop', 'organize'] },
  { id: 'flatten', label: 'Flatten', tools: ['flatten'] },
  { id: 'redact', label: 'Redact', tools: ['redact'] },
];

/** Lists the stages the contributors feed, in the fixed export order, each with its contributing sub-tools. */
export const exportSummary = (contributors: readonly Pick<SubTool, 'id' | 'label'>[]): ExportStep[] =>
  exportStages.flatMap(({ id, label, tools }) => {
    const feeding = contributors.filter((tool) => tools.includes(tool.id)).map((tool) => tool.label);
    return feeding.length ? [{ stage: id, label, tools: feeding }] : [];
  });

// Native requests carry form values, text edits, the flatten choice and redaction marks as their own stage sections, beside the scene;
// marks leave the pages and travel in output page order.
export const sceneSections = ({ pages, textEdits }: PdfScene, formValues: FormValue[] = [], flatten: PdfFlattenChoice | null = null): SceneSections => {
  const redactions = pages.flatMap((page, pageIndex) => (page.redactions ?? []).map(({ rect, color }) => ({ pageIndex, rect, color })));
  const scene = { pages: redactions.length ? pages.map(({ redactions: _marks, ...page }) => page) : pages };
  return { scene, ...(formValues.length ? { formValues } : {}), ...(textEdits?.length ? { textEdits } : {}), ...(flatten ? { flatten } : {}), ...(redactions.length ? { redactions } : {}) };
};

export const sceneContributors = (scene: PdfScene, baseline: PdfScene, formValues: readonly FormValue[] = [], flatten: PdfFlattenChoice | null = null): SubTool[] =>
  subTools.filter((tool) => tool.contributes(scene, baseline, formValues, flatten));

export const exportSuffix = (contributors: readonly Pick<SubTool, 'suffix'>[]): string | null =>
  contributors.length === 0 ? null : contributors.length === 1 ? contributors[0].suffix : combinedSuffix;

/** Plans one export. `outputPath` is the Export As file chosen in the save step; without it the export takes the
 * default name (original stem + suffix, then -1, -2) in `outputLocation`. */
export function planSceneExport({ paths, scene, baseline, outputLocation, outputPath = null, formValues = [], flatten = null }: {
  paths: string[]; scene: PdfScene; baseline: PdfScene; outputLocation: OutputLocation; outputPath?: string | null; formValues?: FormValue[]; flatten?: PdfFlattenChoice | null;
}): SceneExportPlan {
  const contributors = sceneContributors(scene, baseline, formValues, flatten);
  return {
    request: { paths, ...sceneSections(scene, formValues, flatten), outputLocation, ...(outputPath ? { outputPath } : {}) },
    contributors: contributors.map((tool) => tool.id),
    summary: exportSummary(contributors),
    suffix: exportSuffix(contributors),
  };
}
