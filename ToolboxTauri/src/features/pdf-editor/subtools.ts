import type { OutputLocation } from '../../contracts';
import type { PdfFormValue as FormValue } from './contracts';
import type { PdfScene, SceneObject } from './scene';

export type SubToolId = 'form' | SceneObject['kind'] | 'crop' | 'organize';
export interface SubTool {
  id: SubToolId;
  label: string;
  group: 'overlay' | 'document';
  suffix: string;
  contributes: (scene: PdfScene, baseline: PdfScene, formValues: readonly FormValue[]) => boolean;
}
export interface SceneExportRequest { paths: string[]; scene: PdfScene; outputLocation: OutputLocation; formValues?: FormValue[] }
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
];

export const sceneContributors = (scene: PdfScene, baseline: PdfScene, formValues: readonly FormValue[] = []): SubTool[] =>
  subTools.filter((tool) => tool.contributes(scene, baseline, formValues));

export const exportSuffix = (contributors: readonly Pick<SubTool, 'suffix'>[]): string | null =>
  contributors.length === 0 ? null : contributors.length === 1 ? contributors[0].suffix : combinedSuffix;

export function planSceneExport({ paths, scene, baseline, outputLocation, formValues = [] }: {
  paths: string[]; scene: PdfScene; baseline: PdfScene; outputLocation: OutputLocation; formValues?: FormValue[];
}): SceneExportPlan {
  const contributors = sceneContributors(scene, baseline, formValues);
  const request: SceneExportRequest = formValues.length ? { paths, scene, outputLocation, formValues } : { paths, scene, outputLocation };
  return { request, contributors: contributors.map((tool) => tool.id), suffix: exportSuffix(contributors) };
}
