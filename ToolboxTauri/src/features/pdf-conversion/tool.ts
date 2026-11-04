import type { ReactNode } from "react";
import type { RunContext } from "../../components/ToolScaffold";
import type { OutputLocation, ToolDefinition, ToolResult } from "../../contracts";
import type { PdfDocument } from "../pdf-editor";

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
  inspectedDocument: PdfDocument | null;
  selectedPages: number[];
  pageRange: string;
  choosePageRange: (value: string) => void;
  picks: PagePicks;
  pageCounts: Record<string, PageCount>;
};

export interface ConversionTool<Options = unknown> {
  initialOptions?: Options;
  inspectsPdf: boolean;
  initialPages?(document: PdfDocument): number[];
  outputPreview(options: Options, selectedPages: number[]): string;
  exportBlocked?(options: Options, files: string[], pageRange: string): boolean;
  pagesPerPdf?(options: Options, pageRange: string, pageCount: number): number[] | null;
  RunSummary?(props: BatchPagesProps<Options>): ReactNode;
  OptionsPanel?(props: OptionsPanelProps<Options>): ReactNode;
  exampleName?(inputPath: string, utility: ToolDefinition, options: Options): string;
  exampleHint?: string;
  request(paths: string[], outputLocation: OutputLocation, run: RunContext, options: Options, selection: PageSelection): Promise<ToolResult>;
}

export const formatPageRange = (pages: number[]) => {
  const oneBased = [...pages].sort((left, right) => left - right).map((page) => page + 1);
  const ranges: string[] = [];
  for (const page of oneBased) {
    const previous = ranges[ranges.length - 1];
    if (!previous) {
      ranges.push(String(page));
      continue;
    }
    const [start, end] = previous.split("-").map(Number);
    const last = end ?? start;
    if (page === last + 1) ranges[ranges.length - 1] = `${start}-${page}`;
    else ranges.push(String(page));
  }
  return ranges.join(",");
};

export const inspectedPages = ({ document, selectedPages }: PageSelection) => document ? selectedPages : null;
