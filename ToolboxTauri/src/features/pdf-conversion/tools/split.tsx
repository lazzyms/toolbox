import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import type { ConversionTool } from "../tool";

type SplitMode = "pages" | "ranges" | "chunks";
type Options = { splitMode: SplitMode; chunkSize: number };

const rangeSelectionIssue = ({ splitMode }: Options, pageRange: string) => splitMode === "ranges" && !pageRange.trim()
  ? "Enter at least one page range to split by ranges."
  : null;

export const split: ConversionTool<Options> = {
  initialOptions: { splitMode: "pages", chunkSize: 2 },
  inspectsPdf: false,
  outputPreview: () => "Output preview unavailable. Export creates separate PDF files using the selected split mode.",
  exportBlocked: (options, _files, pageRange) => rangeSelectionIssue(options, pageRange) !== null,
  OptionsPanel: ({ options, setOptions, pageRange, choosePageRange }) => {
    const { splitMode, chunkSize } = options;
    const issue = rangeSelectionIssue(options, pageRange);
    return (
      <>
        <div className="workspace-field-grid">
          <div className="workspace-field">
            <Label htmlFor="split-mode">Split mode</Label>
            <NativeSelect id="split-mode" aria-label="Split mode" value={splitMode} onChange={(event) => setOptions({ ...options, splitMode: event.target.value as SplitMode })}>
              <option value="pages">Every page</option>
              <option value="ranges">Ranges</option>
              <option value="chunks">Fixed-size chunks</option>
            </NativeSelect>
          </div>
          {splitMode === "ranges" && (
            <div className="workspace-field">
              <Label htmlFor="split-ranges">Ranges</Label>
              <Input id="split-ranges" aria-label="Page ranges" value={pageRange} onChange={(event) => choosePageRange(event.target.value)} placeholder="1-3, 4-8, 9-" />
            </div>
          )}
          {splitMode === "chunks" && (
            <div className="workspace-field">
              <Label htmlFor="split-chunk-size">Pages per file</Label>
              <Input id="split-chunk-size" aria-label="Pages per file" type="number" min="1" value={chunkSize} onChange={(event) => setOptions({ ...options, chunkSize: Number(event.target.value) })} />
            </div>
          )}
        </div>
        {issue && <p className="workspace-note" role="alert">{issue}</p>}
      </>
    );
  },
  request: (paths, outputLocation, { runId }, { splitMode, chunkSize }, { pageRange }) => invoke<ToolResult>("split_pdf", { runId,
    request: {
      paths,
      pages: [],
      pageRanges: splitMode === "ranges" ? pageRange || null : null,
      splitMode,
      chunkSize: splitMode === "chunks" ? chunkSize : null,
      outputLocation,
    },
  }),
};
