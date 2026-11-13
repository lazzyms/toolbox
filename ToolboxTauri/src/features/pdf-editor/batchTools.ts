import type { ReactNode } from "react";
import type { AtomicToolId, JobOutcome, OutputLocation } from "../../contracts";
import { privacyCheckTool } from "./PrivacyReport";

/** A PDF Editor rail tool that runs on many files through the shared batch shell. */
export interface PdfEditorBatchTool {
  description: string;
  request: (paths: string[], outputLocation: OutputLocation) => Record<string, unknown>;
  /**
   * Marks a read-only tool that saves nothing. It runs under this label, shows the note
   * where other tools show the destination, and renders its results as a report.
   */
  report?: {
    runLabel: (fileCount: number) => string;
    note: string;
    render: (results: JobOutcome[]) => ReactNode;
  };
}

export type PdfEditorBatchTools = Partial<Record<AtomicToolId, PdfEditorBatchTool>>;

export const pdfEditorBatchTools: PdfEditorBatchTools = {
  "pdf-privacy": privacyCheckTool,
};
