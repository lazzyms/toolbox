import { invoke } from "@tauri-apps/api/core";
import { CircleAlert } from "lucide-react";
import type { ToolResult } from "../../../contracts";
import { splitRangesIssue } from "../../../shared/workspaceValidation";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { ConversionTool } from "../tool";

type SplitMode = "ranges" | "everyN";
type Options = { mode: SplitMode; ranges: string; pagesPerFile: string };

const pagesPerFileIssue = (value: string) => /^\d+$/.test(value.trim()) && Number(value) >= 1 ? null : "Enter 1 or more pages per file.";
const splitIssue = ({ mode, ranges, pagesPerFile }: Options) => mode === "ranges" ? splitRangesIssue(ranges) : pagesPerFileIssue(pagesPerFile);
const stemOf = (path: string) => (path.split(/[\\/]/).pop() ?? path).replace(/\.[^.]+$/, "");
const splitStem = (files: string[]) => (files[0] ? stemOf(files[0]) : "") || "document";

/** Split parts are named by the one-based pages they hold, the same way in both modes. */
const splitPartName = (stem: string, first: number, last: number) => `${stem}-${first === last ? `page-${first}` : `pages-${first}-${last}`}.pdf`;

/** Example part names for the File names note: two ranges, or the first two every-N parts. */
const splitExampleNames = (stem: string, mode: SplitMode, pagesPerFile: string): string[] => {
  if (mode === "ranges") return [splitPartName(stem, 1, 3), splitPartName(stem, 5, 5)];
  if (pagesPerFileIssue(pagesPerFile)) return [];
  const size = Number(pagesPerFile);
  return [splitPartName(stem, 1, size), splitPartName(stem, size + 1, size * 2)];
};

export const split: ConversionTool<Options> = {
  initialOptions: { mode: "ranges", ranges: "", pagesPerFile: "1" },
  inspectsPdf: false,
  outputPreview: () => "Output preview unavailable. Export creates one PDF for each range, or for every N pages. Ranges are clamped to each PDF's page count.",
  exportBlocked: (options) => splitIssue(options) !== null,
  exampleName: (inputPath, _utility, { mode, pagesPerFile }) => splitExampleNames(stemOf(inputPath), mode, pagesPerFile)[0] ?? splitPartName(stemOf(inputPath), 1, 1),
  OptionsPanel: ({ options, setOptions, files }) => {
    const { mode, ranges, pagesPerFile } = options;
    const issue = splitIssue(options);
    const names = splitExampleNames(splitStem(files), mode, pagesPerFile);
    return (
      <div className="grid gap-4">
        <span id="split-by-label" className="text-sm leading-none font-medium">Split by</span>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-labelledby="split-by-label"
          className="self-start"
          value={mode}
          onValueChange={(value) => {
            if (value) setOptions({ ...options, mode: value as SplitMode });
          }}
        >
          <ToggleGroupItem value="ranges">Page ranges</ToggleGroupItem>
          <ToggleGroupItem value="everyN">Every N pages</ToggleGroupItem>
        </ToggleGroup>
        {mode === "ranges" ? (
          <div className="workspace-field">
            <Label htmlFor="split-ranges">Page ranges</Label>
            <Input
              id="split-ranges"
              value={ranges}
              onChange={(event) => setOptions({ ...options, ranges: event.target.value })}
              placeholder="1-3, 5, 8-"
              spellCheck={false}
              aria-invalid={issue !== null || undefined}
              aria-describedby={issue ? "split-ranges-help split-options-error" : "split-ranges-help"}
            />
            <p id="split-ranges-help" className="workspace-note">Each range becomes one file. 8- runs to the last page.</p>
          </div>
        ) : (
          <div className="workspace-field">
            <Label htmlFor="split-pages-per-file">Pages per file</Label>
            <div className="flex items-center gap-2">
              <Input
                id="split-pages-per-file"
                className="w-24"
                type="number"
                min="1"
                step="1"
                value={pagesPerFile}
                onChange={(event) => setOptions({ ...options, pagesPerFile: event.target.value })}
                aria-invalid={issue !== null || undefined}
                aria-describedby={issue ? "split-pages-help split-options-error" : "split-pages-help"}
              />
              <span className="workspace-note">pages</span>
            </div>
            <p id="split-pages-help" className="workspace-note">1 makes one file per page. The last file can be shorter.</p>
          </div>
        )}
        {issue && (
          <p id="split-options-error" role="alert" className="flex items-start gap-1.5 text-sm text-destructive">
            <CircleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
            <span>{issue}</span>
          </p>
        )}
        <div className="workspace-field" role="group" aria-labelledby="split-fitting-label">
          <span id="split-fitting-label" className="text-sm leading-none font-medium">Fitting each PDF</span>
          <p className="workspace-note">A range past the last page is shortened or dropped. A PDF is skipped when nothing is left or the result is the whole file.</p>
        </div>
        {names.length > 0 && (
          <div className="workspace-field" role="group" aria-labelledby="split-names-label">
            <span id="split-names-label" className="text-sm leading-none font-medium">File names</span>
            {names.map((name) => <p key={name} className="workspace-note">{name}</p>)}
          </div>
        )}
      </div>
    );
  },
  request: (paths, outputLocation, { runId }, { mode, ranges, pagesPerFile }) => invoke<ToolResult>("split_pdf", { runId,
    request: {
      paths,
      mode: mode === "ranges" ? { kind: "ranges", ranges } : { kind: "everyN", pages: Number(pagesPerFile) },
      outputLocation,
    },
  }),
};
