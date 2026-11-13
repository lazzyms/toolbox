import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { exampleOutputName } from "../components/RunDestination";
import type { ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";

type ExtractMode = "text" | "images" | "both";
type PageScope = "all" | "range";

const MIN_IMAGE_SIZE = { initial: 64, min: 1, max: 2000 };

const modeHints: Record<ExtractMode, string> = {
  text: "One text file for each PDF, in reading order.",
  images: "One folder of images for each PDF.",
  both: "A text file and a folder of images for each PDF.",
};

const exampleHints: Record<ExtractMode, string> = {
  text: "UTF-8 text, a page break between pages. An existing name gets -1, -2 and so on.",
  images: "JPEG and JPEG 2000 are copied as they are; others become lossless PNG. An existing folder is never reused: the new one gets -1, -2 and so on.",
  both: "An existing file or folder is never replaced: the new one gets -1, -2 and so on.",
};

// The same range applies to every PDF, so only its syntax can be checked before the run.
export const pageRangeIssue = (value: string): string | null => {
  if (!value.trim()) return "Enter the pages to extract, like 1-3, 5, 8-.";
  for (const token of value.split(",").map((part) => part.trim())) {
    if (!token) return "Remove the empty entry between commas.";
    const match = /^(\d+)\s*(?:-\s*(\d*))?$/.exec(token);
    if (!match) return `${token} isn't a page or a range. Use numbers, commas and dashes.`;
    const start = Number(match[1]);
    const end = match[2] ? Number(match[2]) : null;
    if (start < 1 || end === 0) return "Pages start at 1.";
    if (end !== null && end < start) return `${token} runs backwards. Write it as ${end}-${start}.`;
  }
  return null;
};

const pdfCount = (count: number) => `${count} ${count === 1 ? "PDF" : "PDFs"}`;

export const ExtractFromPdfView = ({ utility, initialPaths = [], onFilesChange }: { utility: ToolDefinition; initialPaths?: readonly string[]; onFilesChange?: (paths: readonly string[]) => void }) => {
  const [mode, setMode] = useState<ExtractMode>("both");
  const [pageScope, setPageScope] = useState<PageScope>("all");
  const [pageRange, setPageRange] = useState("");
  const [preserveLayout, setPreserveLayout] = useState(false);
  const [skipSmallImages, setSkipSmallImages] = useState(false);
  const [minImageSize, setMinImageSize] = useState(String(MIN_IMAGE_SIZE.initial));

  const wantsText = mode !== "images";
  const wantsImages = mode !== "text";
  const rangeIssue = pageScope === "range" ? pageRangeIssue(pageRange) : null;
  const minSize = Number(minImageSize);
  const minSizeIssue = wantsImages && skipSmallImages && !(Number.isInteger(minSize) && minSize >= MIN_IMAGE_SIZE.min && minSize <= MIN_IMAGE_SIZE.max)
    ? `Enter a whole number from ${MIN_IMAGE_SIZE.min} to ${MIN_IMAGE_SIZE.max}.`
    : null;

  return (
    <ToolScaffold
      key={initialPaths.join("\0") || "no-initial-pdf"}
      variant="workspace"
      sessionKey="pdf-convert"
      utility={utility}
      initialPaths={initialPaths}
      onFilesChange={onFilesChange}
      destinationMentionsSettingKept
      exampleName={mode === "images" ? (path) => exampleOutputName(path, utility.command, "-images", "") : undefined}
      onRun={(paths, outputLocation, { runId }) => invoke<ToolResult>("extract_from_pdf", {
        runId,
        request: {
          paths,
          mode,
          pageRange: pageScope === "range" ? pageRange.trim() : null,
          outputLocation,
          preserveLayout: wantsText && preserveLayout,
          ...(wantsImages && skipSmallImages ? { minImageSize: minSize } : {}),
        },
      })}
    >
      {({ files, run, loading, destination }) => {
        const runLabel = files.length === 0 ? "Extract" : `Extract from ${pdfCount(files.length)}`;
        const blocker = files.length === 0 ? "Add PDFs to start" : rangeIssue ? "Fix the page range first" : minSizeIssue ? "Fix the image size first" : null;
        return (
          <Card className="workspace-control-panel py-0">
            <CardContent className="workspace-control-panel-content grid gap-4 p-4">
              <div>
                <h2 className="workspace-active-command">{utility.title}</h2>
                <p className="workspace-panel-copy">{utility.blurb}</p>
              </div>
              <fieldset className="workspace-field" disabled={loading}>
                <legend className="workspace-panel-label">Extract</legend>
                <ToggleGroup
                  type="single"
                  variant="outline"
                  size="sm"
                  aria-label="Extract"
                  aria-describedby="extract-mode-hint"
                  value={mode}
                  disabled={loading}
                  onValueChange={(value) => {
                    if (value) setMode(value as ExtractMode);
                  }}
                >
                  <ToggleGroupItem value="text">Text</ToggleGroupItem>
                  <ToggleGroupItem value="images">Images</ToggleGroupItem>
                  <ToggleGroupItem value="both">Both</ToggleGroupItem>
                </ToggleGroup>
                <p id="extract-mode-hint" className="workspace-note">{modeHints[mode]}</p>
              </fieldset>
              <fieldset className="workspace-field" disabled={loading}>
                <legend className="workspace-panel-label">Pages</legend>
                <ToggleGroup
                  type="single"
                  variant="outline"
                  size="sm"
                  aria-label="Pages"
                  value={pageScope}
                  disabled={loading}
                  onValueChange={(value) => {
                    if (value) setPageScope(value as PageScope);
                  }}
                >
                  <ToggleGroupItem value="all">All pages</ToggleGroupItem>
                  <ToggleGroupItem value="range">Range</ToggleGroupItem>
                </ToggleGroup>
                {pageScope === "range" && (
                  <>
                    <Label htmlFor="extract-page-range" className="sr-only">Page range for every PDF</Label>
                    <Input
                      id="extract-page-range"
                      value={pageRange}
                      placeholder="1-3, 5, 8-"
                      aria-invalid={rangeIssue !== null && pageRange.trim() !== ""}
                      aria-describedby={rangeIssue && pageRange.trim() ? "extract-range-error extract-range-hint" : "extract-range-hint extract-range-scope"}
                      onChange={(event) => setPageRange(event.target.value)}
                    />
                    {rangeIssue && pageRange.trim() && <p id="extract-range-error" className="workspace-note" role="alert">{rangeIssue}</p>}
                    <p id="extract-range-hint" className="workspace-note">Commas and dashes, like 1-3, 5, 8-. 8- means page 8 to the end.</p>
                  </>
                )}
                <p id="extract-range-scope" className="workspace-note">
                  {pageScope === "range" ? "One range applies to every PDF." : "Every page of every PDF."}
                </p>
              </fieldset>
              {wantsText && (
                <fieldset className="workspace-field" disabled={loading}>
                  <legend className="workspace-panel-label">Text</legend>
                  <Label className="workspace-check" htmlFor="extract-preserve-layout">
                    <Checkbox
                      id="extract-preserve-layout"
                      aria-describedby="extract-layout-hint"
                      checked={preserveLayout}
                      disabled={loading}
                      onCheckedChange={(checked) => setPreserveLayout(checked === true)}
                    />
                    <span>Preserve layout</span>
                  </Label>
                  <p id="extract-layout-hint" className="workspace-note">Keeps columns and tables lined up. Off follows reading order.</p>
                </fieldset>
              )}
              {wantsImages && (
                <fieldset className="workspace-field" disabled={loading}>
                  <legend className="workspace-panel-label">Images</legend>
                  <Label className="workspace-check" htmlFor="extract-skip-small">
                    <Checkbox
                      id="extract-skip-small"
                      aria-describedby="extract-min-hint"
                      checked={skipSmallImages}
                      disabled={loading}
                      onCheckedChange={(checked) => setSkipSmallImages(checked === true)}
                    />
                    <span>Skip small images</span>
                  </Label>
                  <p id="extract-min-hint" className="workspace-note">
                    {skipSmallImages ? "Leaves out bullets, lines and spacers. The result counts what was left out." : "Off saves every image at its original size."}
                  </p>
                  {skipSmallImages && (
                    <div className="flex items-center gap-2">
                      <Label htmlFor="extract-min-size" className="whitespace-nowrap">Smaller than</Label>
                      <Input
                        id="extract-min-size"
                        type="number"
                        min={MIN_IMAGE_SIZE.min}
                        max={MIN_IMAGE_SIZE.max}
                        className="w-20"
                        value={minImageSize}
                        aria-invalid={minSizeIssue !== null}
                        aria-describedby="extract-min-hint"
                        onChange={(event) => setMinImageSize(event.target.value)}
                      />
                      <span className="workspace-note">px wide or tall</span>
                    </div>
                  )}
                  {minSizeIssue && <p className="workspace-note" role="alert">{minSizeIssue}</p>}
                </fieldset>
              )}
              {destination}
              <p className="workspace-note">{exampleHints[mode]}</p>
              <p className="workspace-note">Originals are only read, never changed.{wantsText && wantsImages ? " A PDF with nothing to extract is skipped." : ""}</p>
              {blocker && <p id="extract-run-blocker" className="workspace-note">{blocker}</p>}
              <Button
                variant="default"
                aria-label={runLabel}
                aria-describedby={blocker ? "extract-run-blocker" : undefined}
                disabled={loading || blocker !== null}
                onClick={run}
                className="workspace-primary-action"
                {...shortcutTarget("run")}
              >
                {runLabel}
                <ShortcutHint id="run" />
              </Button>
            </CardContent>
          </Card>
        );
      }}
    </ToolScaffold>
  );
};
