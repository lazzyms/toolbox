import type { OutputLocation } from '../../contracts';
import type { PdfTextEdit } from './contracts';
import type { PdfScene, SceneObject, SceneRect } from './scene';

export type SubToolId = SceneObject['kind'] | 'edit-text' | 'crop' | 'organize' | 'redact';
export interface SubTool {
  id: SubToolId;
  label: string;
  group: 'overlay' | 'document';
  suffix: string;
  contributes: (scene: PdfScene, baseline: PdfScene) => boolean;
}
export interface RedactionMark { pageIndex: number; rect: SceneRect; color: string }
export interface SceneSections { scene: Pick<PdfScene, 'pages'>; textEdits?: PdfTextEdit[]; redactions?: RedactionMark[] }
export interface SceneExportRequest extends SceneSections { paths: string[]; outputLocation: OutputLocation; outputPath?: string }
export interface SceneExportPlan { request: SceneExportRequest; contributors: SubToolId[]; suffix: string | null }

export const combinedSuffix = '-edited';

const overlay = (id: SceneObject['kind'], label: string): SubTool => ({
  id, label, group: 'overlay', suffix: '-edited',
  contributes: (scene) => scene.pages.some((page) => page.objects.some((object) => object.kind === id)),
});
const pageLayout = (scene: PdfScene) => JSON.stringify(scene.pages.map((page) => [page.id, page.rotation]));

export const subTools: readonly SubTool[] = [
  { id: 'edit-text', label: 'Edit text', group: 'document', suffix: '-edited',
    contributes: (scene) => Boolean(scene.textEdits?.length) },
  overlay('text', 'Text'),
  overlay('highlight', 'Highlight'),
  overlay('shape', 'Shape'),
  overlay('signature', 'Signature'),
  overlay('watermark', 'Watermark'),
  { id: 'crop', label: 'Crop', group: 'document', suffix: '-edited',
    contributes: (scene) => scene.pages.some((page) => page.crop !== null) },
  { id: 'organize', label: 'Organize pages', group: 'document', suffix: '-edited',
    contributes: (scene, baseline) => pageLayout(scene) !== pageLayout(baseline) },
  { id: 'redact', label: 'Redact', group: 'document', suffix: '-redacted',
    contributes: (scene) => scene.pages.some((page) => page.redactions?.length) },
];

// Native requests carry text edits and redaction marks as their own stage sections, beside the scene;
// marks leave the pages and travel in output page order.
export const sceneSections = ({ pages, textEdits }: PdfScene): SceneSections => {
  const redactions = pages.flatMap((page, pageIndex) => (page.redactions ?? []).map(({ rect, color }) => ({ pageIndex, rect, color })));
  const scene = { pages: redactions.length ? pages.map(({ redactions: _marks, ...page }) => page) : pages };
  return { scene, ...(textEdits?.length ? { textEdits } : {}), ...(redactions.length ? { redactions } : {}) };
};

export const sceneContributors = (scene: PdfScene, baseline: PdfScene): SubTool[] =>
  subTools.filter((tool) => tool.contributes(scene, baseline));

export const exportSuffix = (contributors: readonly Pick<SubTool, 'suffix'>[]): string | null =>
  contributors.length === 0 ? null : contributors.length === 1 ? contributors[0].suffix : combinedSuffix;

/** Plans one export. `outputPath` is the Export As file chosen in the save step; without it the export takes the
 * default name (original stem + suffix, then -1, -2) in `outputLocation`. */
export function planSceneExport({ paths, scene, baseline, outputLocation, outputPath = null }: {
  paths: string[]; scene: PdfScene; baseline: PdfScene; outputLocation: OutputLocation; outputPath?: string | null;
}): SceneExportPlan {
  const contributors = sceneContributors(scene, baseline);
  return {
    request: { paths, ...sceneSections(scene), outputLocation, ...(outputPath ? { outputPath } : {}) },
    contributors: contributors.map((tool) => tool.id),
    suffix: exportSuffix(contributors),
  };
}
