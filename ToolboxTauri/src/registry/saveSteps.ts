export interface SaveDialog {
  title: string;
  extension: string;
}

export interface ManyToOneSaveStep extends SaveDialog {
  runLabel: string;
  minimumInputs: number;
  summary: (inputCount: number) => string;
}

const count = (value: number, one: string, many: string) => `${value} ${value === 1 ? one : many}`;

const manyToOneSaveSteps: Record<string, ManyToOneSaveStep> = {
  merge_pdfs: { title: "Save the merged PDF", extension: "pdf", runLabel: "Merge…", minimumInputs: 1, summary: (n) => `${count(n, "file", "files")}, joined top to bottom.` },
  images_to_pdf: { title: "Save the combined PDF", extension: "pdf", runLabel: "Make PDF…", minimumInputs: 1, summary: (n) => `${count(n, "image", "images")}, one per page.` },
  // A comparison reads exactly two PDFs and writes one text report.
  compare_pdfs: { title: "Save the comparison report", extension: "txt", runLabel: "Compare…", minimumInputs: 2, summary: () => "2 PDFs, compared line by line." },
  create_gif: { title: "Save the GIF", extension: "gif", runLabel: "Make GIF…", minimumInputs: 1, summary: (n) => `${count(n, "frame", "frames")}, in the order shown.` },
  // One TIFF splits into pages, so only a combine of two or more makes one file.
  process_tiff_pages: { title: "Save the combined TIFF", extension: "tiff", runLabel: "Make TIFF…", minimumInputs: 2, summary: (n) => `${count(n, "TIFF file", "TIFF files")}, combined in the order shown.` },
};

export const saveStepFor = (tool: { command: string; capability: { manyToOne: boolean } }): ManyToOneSaveStep | null => {
  if (!tool.capability.manyToOne) return null;
  const step = manyToOneSaveSteps[tool.command];
  if (!step) throw new Error(`${tool.command} makes one file but has no save step.`);
  return step;
};

export const saveStepAppliesTo = (step: ManyToOneSaveStep | null, inputCount: number): step is ManyToOneSaveStep =>
  step !== null && inputCount >= step.minimumInputs;

export const saveStepCommands = () => Object.keys(manyToOneSaveSteps);
