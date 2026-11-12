import type { ReactNode } from "react";
import type { RunContext } from "../../components/ToolScaffold";
import type { AtomicToolId, OutputLocation, ToolDefinition, ToolResult } from "../../contracts";
import type { EntrySortOption, IntakeCopy } from "../../components/intake";
import type { PdfDocument } from "../pdf-editor";
import { formatPageSpans, type PageSpan } from "../../shared/pageRanges";

export type PageSelection = { document: PdfDocument | null; selectedPages: number[]; pageRange: string; picks: PagePicks };

export type PagePicks = Record<string, number[]>;

export type PageCount = number | "pending" | "locked" | "unreadable";

export type BatchPagesProps<Options> = {
  options: Options;
  files: string[];
  pageRange: string;
  picks: PagePicks;
  pageCounts: Record<string, PageCount>;
};

export type OptionsPanelProps<Options> = {
  options: Options;
  setOptions(options: Options): void;
  files: string[];
  /** The file list row the user last selected, for previews of one input. */
  selectedFileIndex: number;
  inspectedDocument: PdfDocument | null;
  selectedPages: number[];
  pageRange: string;
  choosePageRange: (value: string) => void;
  /** True when the range was written by picking thumbnails rather than typed. */
  pageRangeFromPicks: boolean;
  picks: PagePicks;
  pageCounts: Record<string, PageCount>;
  /** True while a run is in flight, so a panel can disable its inputs. */
  loading: boolean;
  onSelectTool?: (id: AtomicToolId) => void;
};

export interface ConversionTool<Options = unknown> {
  initialOptions?: Options;
  /** Options to start from when another tool hands its files over. */
  handoffOptions?: Options;
  inspectsPdf: boolean;
  /**
   * One typed range applies to every file in the batch and is shortened to each file's
   * length natively. Thumbnails are offered only for a single PDF, picking writes the
   * range, and picking nothing means every page.
   */
  batchPageRange?: boolean;
  sortOptions?: readonly EntrySortOption[];
  /** Intake wording for a tool whose list flags files that will be skipped or aren't available yet. */
  intakeCopy?(utility: ToolDefinition): IntakeCopy;
  initialPages?(document: PdfDocument): number[];
  outputPreview(options: Options, selectedPages: number[]): string;
  exportBlocked?(options: Options, files: string[], pageRange: string): boolean;
  pagesPerPdf?(options: Options, pageRange: string, pageCount: number): number[] | null;
  RunSummary?(props: BatchPagesProps<Options>): ReactNode;
  /** A status shown above the export action while the list can't be exported yet. */
  exportNote?(files: string[]): string | null;
  OptionsPanel?(props: OptionsPanelProps<Options>): ReactNode;
  exampleName?(inputPath: string, utility: ToolDefinition, options: Options): string;
  exampleHint?: string;
  request(paths: string[], outputLocation: OutputLocation, run: RunContext, options: Options, selection: PageSelection): Promise<ToolResult>;
}

export const formatPageRange = (pages: number[]) => {
  const spans: PageSpan[] = [];
  for (const page of [...new Set(pages)].sort((left, right) => left - right).map((index) => index + 1)) {
    const previous = spans[spans.length - 1];
    if (previous?.last === page - 1) previous.last = page;
    else spans.push({ first: page, last: page });
  }
  return formatPageSpans(spans);
};

export const inspectedPages = ({ document, selectedPages }: PageSelection) => document ? selectedPages : null;
