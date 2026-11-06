import type { AtomicToolId, OutputLocation } from "../../contracts";

/** A PDF Editor rail tool that runs on many files through the shared batch shell. */
export interface PdfEditorBatchTool {
  description: string;
  request: (paths: string[], outputLocation: OutputLocation) => Record<string, unknown>;
}

export type PdfEditorBatchTools = Partial<Record<AtomicToolId, PdfEditorBatchTool>>;

export const pdfEditorBatchTools: PdfEditorBatchTools = {};
