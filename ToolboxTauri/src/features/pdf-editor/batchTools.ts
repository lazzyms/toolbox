import type { ReactNode } from "react";
import type { AtomicToolId, JobOutcome, OutputLocation, PasswordSet } from "../../contracts";
import type { ReportRun } from "../../components/ToolScaffold";
import type { ExistingPdfUtilityId } from "../../views/PDFEditorWorkspaceView";
import { privacyCheckTool } from "./PrivacyReport";

/** Opens another tool on the file a report describes, and saves reports to this run's destination. */
export interface ReportActions extends Omit<ReportRun, "withPassword"> {
  canOpen: (tool: ExistingPdfUtilityId) => boolean;
  open: (tool: ExistingPdfUtilityId, path: string) => void;
  /** Runs one file again with a password for that run only. Present when the tool takes passwords. */
  retryWithPassword?: (inputPath: string, password: string) => Promise<void>;
}

/** A PDF Editor rail tool that runs on many files through the shared batch shell. */
export interface PdfEditorBatchTool {
  description: string;
  request: (paths: string[], outputLocation: OutputLocation, passwords: PasswordSet) => Record<string, unknown>;
  /** Shows the shared batch password with per-file overrides, tried only on files that need an open password. */
  openPassword?: boolean;
  /**
   * Marks a read-only tool. It runs under this label, describes where its reports save
   * beside the note, and renders its results as a report.
   */
  report?: {
    runLabel: (fileCount: number) => string;
    note: string;
    destination: (files: string[]) => { label: string; note: string };
    render: (results: JobOutcome[], actions: ReportActions) => ReactNode;
  };
}

export type PdfEditorBatchTools = Partial<Record<AtomicToolId, PdfEditorBatchTool>>;

export const pdfEditorBatchTools: PdfEditorBatchTools = {
  "pdf-privacy": privacyCheckTool,
};
