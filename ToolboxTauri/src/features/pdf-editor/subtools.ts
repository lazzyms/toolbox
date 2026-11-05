import type { OutputLocation } from '../../contracts';
import type { PdfScene, SceneObject } from './scene';

export type SubToolId = SceneObject['kind'] | 'crop' | 'organize';
export interface SubTool {
  id: SubToolId;
  label: string;
  group: 'overlay' | 'document';
  suffix: string;
  contributes: (scene: PdfScene, baseline: PdfScene) => boolean;
}
export interface SceneExportRequest { paths: string[]; scene: PdfScene; outputLocation: OutputLocation }
export interface SceneExportPlan { request: SceneExportRequest; contributors: SubToolId[]; suffix: string | null }

export const combinedSuffix = '-edited';

const overlay = (id: SceneObject['kind'], label: string): SubTool => ({
  id, label, group: 'overlay', suffix: '-edited',
  contributes: (scene) => scene.pages.some((page) => page.objects.some((object) => object.kind === id)),
});
const pageLayout = (scene: PdfScene) => JSON.stringify(scene.pages.map((page) => [page.id, page.rotation]));

export const subTools: readonly SubTool[] = [
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
];

export const sceneContributors = (scene: PdfScene, baseline: PdfScene): SubTool[] =>
  subTools.filter((tool) => tool.contributes(scene, baseline));

export const exportSuffix = (contributors: readonly Pick<SubTool, 'suffix'>[]): string | null =>
  contributors.length === 0 ? null : contributors.length === 1 ? contributors[0].suffix : combinedSuffix;

export function planSceneExport({ paths, scene, baseline, outputLocation }: {
  paths: string[]; scene: PdfScene; baseline: PdfScene; outputLocation: OutputLocation;
}): SceneExportPlan {
  const contributors = sceneContributors(scene, baseline);
  return { request: { paths, scene, outputLocation }, contributors: contributors.map((tool) => tool.id), suffix: exportSuffix(contributors) };
}
