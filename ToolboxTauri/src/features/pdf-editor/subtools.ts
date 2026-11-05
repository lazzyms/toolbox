import type { OutputLocation } from '../../contracts';
import type { PdfFlattenChoice, PdfFormValue as FormValue } from './contracts';
import type { PdfScene, SceneObject, SceneRect } from './scene';

export type SubToolId = 'form' | SceneObject['kind'] | 'crop' | 'organize' | 'flatten' | 'redact';
export interface SubTool {
  id: SubToolId;
  label: string;
  group: 'overlay' | 'document';
  suffix: string;
  contributes: (scene: PdfScene, baseline: PdfScene, formValues: readonly FormValue[], flatten: PdfFlattenChoice | null) => boolean;
}
export interface RedactionMark { pageIndex: number; rect: SceneRect; color: string }
export interface SceneExportRequest { paths: string[]; scene: PdfScene; outputLocation: OutputLocation; formValues?: FormValue[]; flatten?: PdfFlattenChoice; redactions?: RedactionMark[] }
export interface SceneExportPlan { request: SceneExportRequest; contributors: SubToolId[]; suffix: string | null }

export const combinedSuffix = '-edited';

const overlay = (id: SceneObject['kind'], label: string): SubTool => ({
  id, label, group: 'overlay', suffix: '-edited',
  contributes: (scene) => scene.pages.some((page) => page.objects.some((object) => object.kind === id)),
});
const pageLayout = (scene: PdfScene) => JSON.stringify(scene.pages.map((page) => [page.id, page.rotation]));

export const subTools: readonly SubTool[] = [
  { id: 'form', label: 'Fill form', group: 'document', suffix: '-filled',
    contributes: (_scene, _baseline, formValues) => formValues.length > 0 },
  overlay('text', 'Text'),
  overlay('highlight', 'Highlight'),
  overlay('shape', 'Shape'),
  overlay('signature', 'Signature'),
  overlay('watermark', 'Watermark'),
  { id: 'crop', label: 'Crop', group: 'document', suffix: '-edited',
    contributes: (scene) => scene.pages.some((page) => page.crop !== null) },
  { id: 'organize', label: 'Organize pages', group: 'document', suffix: '-edited',
    contributes: (scene, baseline) => pageLayout(scene) !== pageLayout(baseline) },
  { id: 'flatten', label: 'Flatten', group: 'document', suffix: '-flattened',
    contributes: (_scene, _baseline, _formValues, flatten) => flatten !== null },
  { id: 'redact', label: 'Redact', group: 'document', suffix: '-redacted',
    contributes: (scene) => scene.pages.some((page) => page.redactions?.length) },
];

export const sceneContributors = (scene: PdfScene, baseline: PdfScene, formValues: readonly FormValue[] = [], flatten: PdfFlattenChoice | null = null): SubTool[] =>
  subTools.filter((tool) => tool.contributes(scene, baseline, formValues, flatten));

export const exportSuffix = (contributors: readonly Pick<SubTool, 'suffix'>[]): string | null =>
  contributors.length === 0 ? null : contributors.length === 1 ? contributors[0].suffix : combinedSuffix;

export function planSceneExport({ paths, scene, baseline, outputLocation, formValues = [], flatten = null }: {
  paths: string[]; scene: PdfScene; baseline: PdfScene; outputLocation: OutputLocation; formValues?: FormValue[]; flatten?: PdfFlattenChoice | null;
}): SceneExportPlan {
  const contributors = sceneContributors(scene, baseline, formValues, flatten);
  const redactions = scene.pages.flatMap((page, pageIndex) => (page.redactions ?? []).map(({ rect, color }) => ({ pageIndex, rect, color })));
  const request: SceneExportRequest = {
    paths,
    scene: redactions.length ? { pages: scene.pages.map(({ redactions: _marks, ...page }) => page) } : scene,
    outputLocation,
    ...(formValues.length ? { formValues } : {}),
    ...(flatten ? { flatten } : {}),
    ...(redactions.length ? { redactions } : {}),
  };
  return { request, contributors: contributors.map((tool) => tool.id), suffix: exportSuffix(contributors) };
}
