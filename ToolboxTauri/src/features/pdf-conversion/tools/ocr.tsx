import { invoke } from "@tauri-apps/api/core";
import { exampleOutputName } from "../../../components/RunDestination";
import type { OcrMode, ToolResult } from "../../../contracts";
import { ocrRequest } from "../../../ocrRequest";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { BUILT_IN_OCR_LANGUAGE, OcrLanguagePicker } from "../../../views/OcrLanguagePicker";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { parsePageRanges } from "../../../shared/pageRanges";
import type { ConversionTool } from "../tool";

type PagesScope = "all" | "range";
type Options = { mode: OcrMode; language: string; force: boolean; pages: PagesScope };

const modeHints: Record<OcrMode, string> = {
  searchablePdf: "Looks exactly like the scan, with text you can select and search.",
  text: "Plain text, page by page. Pages that already have text add it as it is, so the file covers every page in scope.",
};

const rangeHints = {
  all: "Every page of every file. Choose Range to limit them. With one PDF, pick pages by thumbnail.",
  range: "One range applies to every PDF and multi-page TIFF. Pages past a file's end are left out, and a file with no pages left is skipped.",
  picked: "Picking pages writes this range, and typing a range updates the picks.",
};

export const ocr: ConversionTool<Options> = {
  initialOptions: { mode: "searchablePdf", language: BUILT_IN_OCR_LANGUAGE.code, force: false, pages: "all" },
  // Extract from PDF hands scans over because it couldn't read their text, so OCR keeps that ask.
  handoffOptions: { mode: "text", language: BUILT_IN_OCR_LANGUAGE.code, force: false, pages: "all" },
  inspectsPdf: true,
  batchPageRange: true,
  outputPreview: ({ mode }) => `Output preview unavailable. Export recognizes the text on each page and saves it as ${mode === "text" ? "a text file" : "a searchable PDF"}.`,
  OptionsPanel: ({ options, setOptions, pageRange, choosePageRange, pageRangeFromPicks }) => {
    // Picking a thumbnail writes a range, which puts Pages in Range.
    const scope: PagesScope = options.pages === "range" || pageRange ? "range" : "all";
    const parsed = pageRange.trim() ? parsePageRanges(pageRange) : null;
    return (
    <>
    <OcrLanguagePicker value={options.language} onChange={(language) => setOptions({ ...options, language })} />
    <div className="workspace-field">
      <Label id="ocr-save-as-label">Save as</Label>
      <ToggleGroup
        type="single"
        variant="outline"
        size="sm"
        aria-labelledby="ocr-save-as-label"
        value={options.mode}
        onValueChange={(value) => {
          if (value) setOptions({ ...options, mode: value as OcrMode });
        }}
      >
        <ToggleGroupItem value="searchablePdf">Searchable PDF</ToggleGroupItem>
        <ToggleGroupItem value="text">Text file</ToggleGroupItem>
      </ToggleGroup>
      <p className="workspace-panel-copy">{modeHints[options.mode]}</p>
    </div>
    <div className="workspace-field">
      <Label id="ocr-pages-label">Pages</Label>
      <ToggleGroup
        type="single"
        variant="outline"
        size="sm"
        aria-labelledby="ocr-pages-label"
        value={scope}
        onValueChange={(value) => {
          if (!value) return;
          setOptions({ ...options, pages: value as PagesScope });
          if (value === "all") choosePageRange("");
        }}
      >
        <ToggleGroupItem value="all">All pages</ToggleGroupItem>
        <ToggleGroupItem value="range">Range</ToggleGroupItem>
      </ToggleGroup>
      {scope === "range" && (
        <>
          <Label className="sr-only" htmlFor="ocr-page-range">Page range</Label>
          <Input
            id="ocr-page-range"
            value={pageRange}
            aria-describedby="ocr-page-range-hint"
            aria-invalid={parsed?.ok === false}
            onChange={(event) => {
              // Typing keeps Range chosen, even once the field is emptied.
              if (options.pages !== "range") setOptions({ ...options, pages: "range" });
              choosePageRange(event.target.value);
            }}
          />
          <p id="ocr-page-range-hint" className="workspace-panel-copy">Commas and dashes, like 1-3, 5, 8-. 8- means page 8 to the end.</p>
          {parsed?.ok === false && <p className="workspace-note" role="alert">{parsed.message}</p>}
        </>
      )}
      <p className="workspace-panel-copy">{scope === "all" ? rangeHints.all : pageRangeFromPicks ? rangeHints.picked : rangeHints.range}</p>
      {scope === "range" && pageRangeFromPicks && <Badge variant="outline" className="self-start">Set by the page picks</Badge>}
    </div>
    <div className="workspace-field">
      <Label className="workspace-check" htmlFor="ocr-force">
        <Checkbox id="ocr-force" aria-describedby="ocr-force-hint" checked={options.force} onCheckedChange={(checked) => setOptions({ ...options, force: checked === true })} />
        <span>Force: redo earlier OCR text</span>
      </Label>
      <p id="ocr-force-hint" className="workspace-panel-copy">
        Reads pages again when their only text is an invisible layer from an earlier OCR run, and replaces that layer. Pages with visible text are always skipped.
      </p>
    </div>
    </>
    );
  },
  exampleName: (inputPath, utility, { mode }) => exampleOutputName(inputPath, "ocr_pdf", utility.capability.outputSuffix, mode === "text" ? "txt" : "pdf"),
  // The Pages range, typed or written by thumbnail picks, applies to every file; an empty one means every page.
  request: (paths, outputLocation, { runId }, { mode, language, force }, { pageRange }) =>
    invoke<ToolResult>("ocr_pdf", { runId, request: ocrRequest(paths, outputLocation, pageRange, language, mode, force) }),
};
