import type { ReactNode } from "react";
import type { RunContext } from "../../components/ToolScaffold";
import type { AtomicToolId, OutputLocation, ToolDefinition, ToolResult } from "../../contracts";
import type { PdfDocument } from "../pdf-editor";

export type PageSelection = { document: PdfDocument | null; selectedPages: number[]; pageRange: string };

export type OptionsPanelProps<Options> = {
  options: Options;
  setOptions(options: Options): void;
  files: string[];
  inspectedDocument: PdfDocument | null;
  selectedPages: number[];
  pageRange: string;
  choosePageRange: (value: string) => void;
  onSelectTool?: (id: AtomicToolId) => void;
};

export interface ConversionTool<Options = unknown> {
  initialOptions?: Options;
  inspectsPdf: boolean;
  initialPages?(document: PdfDocument): number[];
  outputPreview(options: Options, selectedPages: number[]): string;
  exportBlocked?(options: Options, files: string[], pageRange: string): boolean;
  /** A status shown above the export action while the list can't be exported yet. */
  exportNote?(files: string[]): string | null;
  OptionsPanel?(props: OptionsPanelProps<Options>): ReactNode;
  exampleName?(inputPath: string, utility: ToolDefinition, options: Options): string;
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
