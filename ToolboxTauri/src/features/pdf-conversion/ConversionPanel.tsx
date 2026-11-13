import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { parsePageRange, selectedPageLabel } from "../../shared/workspaceValidation";
import { pagesWithin, parsePageRanges } from "../../shared/pageRanges";
import type { PdfDocument } from "../pdf-editor";
import type { AtomicToolId, ToolDefinition } from "../../contracts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { ShortcutHint } from "../../components/ShortcutHint";
import { workspaceForTool } from "../../registry";
import { shortcutTarget } from "../../shortcuts";
import { listPages } from "./pageRanges";
import { formatPageRange, type ConversionTool, type PageCount, type PagePicks } from "./tool";

type PdfInspectionState =
  | { kind: "idle" }
  | { kind: "pending"; path: string }
  | { kind: "ready"; path: string }
  | { kind: "error"; path: string };

type ConversionPanelProps = {
  destination: ReactNode;
  runLabel: string | null;
  activeUtility: ToolDefinition;
  tool: ConversionTool;
  options: unknown;
  setOptions: (options: unknown) => void;
  files: string[];
  selectedFileIndex: number;
  run: () => Promise<void>;
  loading: boolean;
  pageRange: string;
  setPageRange: (value: string) => void;
  document: PdfDocument | null;
  setDocument: (value: PdfDocument | null) => void;
  selectedPages: number[];
  setSelectedPages: (value: number[]) => void;
  inspectError: string;
  setInspectError: (value: string) => void;
  picks: PagePicks;
  setPicks: (value: PagePicks) => void;
  onSelectTool?: (id: AtomicToolId) => void;
};

const countFailure = (error: unknown): PageCount => /password|encrypt/i.test(String(error)) ? "locked" : "unreadable";

const pagesLabel = (count: number) => `${count} ${count === 1 ? "page" : "pages"}`;

const inWords = (parts: string[]) => parts.length < 2 ? parts.join("") : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;

/** What the picks mean for the one PDF on show, in the board's words. */
const pickStatus = (picks: number[], pageCount: number, pageRange: string, fromPicks: boolean) => {
  if (picks.length) {
    const range = formatPageRange(picks);
    const picked = `${picks.length} of ${pagesLabel(pageCount)} picked: ${inWords(range.split(", "))}.`;
    return fromPicks ? `${picked} The Pages range now says ${range}.` : picked;
  }
  if (pageRange.trim()) return `No pages of this PDF are in ${pageRange.trim()}, so it will be skipped. Pick pages or change the range.`;
  return `No pages picked, so ${pageCount === 1 ? "its 1 page runs" : `all ${pageCount} pages run`}. Pick pages to limit them.`;
};

export const ConversionPanel = ({
  destination,
  runLabel,
  activeUtility,
  tool,
  options,
  setOptions,
  files,
  selectedFileIndex,
  run,
  loading,
  pageRange,
  setPageRange,
  document,
  setDocument,
  selectedPages,
  setSelectedPages,
  inspectError,
  setInspectError,
  picks,
  setPicks,
  onSelectTool,
}: ConversionPanelProps) => {
  const pagesPerPdf = tool.pagesPerPdf;
  const batchRange = tool.batchPageRange === true;
  // A batch-wide range offers thumbnails only while the list holds exactly one PDF, not one image.
  const onePdf = files.length === 1 && files[0].toLowerCase().endsWith(".pdf");
  const inputPath = batchRange ? (onePdf ? files[0] : undefined) : pagesPerPdf ? files[selectedFileIndex] ?? files[0] : files[0];
  const [pageRangeFromPicks, setPageRangeFromPicks] = useState(false);
  const [inspection, setInspection] = useState<PdfInspectionState>({ kind: "idle" });
  const [pageCounts, setPageCounts] = useState<Record<string, PageCount>>({});
  const acceptedExtensions = new Set(activeUtility.capability.acceptedExtensions.map((extension) => extension.toLowerCase()));
  const unsupportedFiles = files.filter((path) => {
    const name = path.split(/[\\/]/).pop() ?? path;
    const extension = name.includes(".") ? `.${name.split(".").pop()?.toLowerCase()}` : "";
    return !acceptedExtensions.has(extension);
  });
  const inputCardinalityIssue = activeUtility.capability.inputCardinality === "single" && files.length > 1
    ? `This action accepts one input file, but ${files.length} are still open. Close the extra files or switch to a multi-file action.`
    : null;
  const inputFormatIssue = unsupportedFiles.length > 0
    ? `${unsupportedFiles.length} ${unsupportedFiles.length === 1 ? "file is" : "files are"} not supported by ${activeUtility.title}. Close ${unsupportedFiles.length === 1 ? "it" : "them"} or switch actions.`
    : null;
  const inputPolicyIssue = inputCardinalityIssue ?? inputFormatIssue;

  useEffect(() => {
    setPageRange("");
    setPageRangeFromPicks(false);
  }, [tool, setPageRange]);

  useEffect(() => {
    let current = true;
    setDocument(null);
    setSelectedPages([]);
    // A batch-wide or per-PDF range belongs to the whole list, so it outlives a change of files.
    if (!pagesPerPdf && !batchRange) setPageRange("");
    setInspectError("");
    if (!inputPath || inputPolicyIssue) {
      setInspection({ kind: "idle" });
      return;
    }
    if (!tool.inspectsPdf) {
      setInspection({ kind: "idle" });
      return () => {
        current = false;
      };
    }
    setInspection({ kind: "pending", path: inputPath });
    void invoke<PdfDocument>("inspect_pdf", { request: { path: inputPath } })
      .then((value) => {
        if (!current) return;
        setDocument(value);
        setSelectedPages(tool.initialPages?.(value) ?? value.pages.map((page) => page.index));
        if (!pagesPerPdf && !batchRange) setPageRange("");
        setInspection({ kind: "ready", path: inputPath });
      })
      .catch((error) => {
        if (!current) return;
        setInspectError(String(error));
        setInspection({ kind: "error", path: inputPath });
      });
    return () => {
      current = false;
    };
  }, [tool, pagesPerPdf, batchRange, inputPath, inputPolicyIssue, setDocument, setInspectError, setPageRange, setSelectedPages]);

  const fileKey = files.join("\0");
  useEffect(() => {
    if (!pagesPerPdf) return;
    let current = true;
    const paths = fileKey ? fileKey.split("\0") : [];
    setPageCounts((counts) => Object.fromEntries(paths.map((path) => [path, counts[path] ?? "pending"])));
    for (const path of new Set(paths)) {
      void invoke<PdfDocument>("inspect_pdf_scene", { request: { path } })
        .then((value): PageCount => value.pages.length)
        .catch(countFailure)
        .then((count) => { if (current) setPageCounts((counts) => path in counts ? { ...counts, [path]: count } : counts); });
    }
    return () => {
      current = false;
    };
  }, [pagesPerPdf, fileKey]);

  const inspectedPicks = inputPath ? picks[inputPath] : undefined;
  const inspectedDocument = inspection.kind === "ready" && inspection.path === inputPath ? document : null;
  // A batch-wide range is checked only for its form here: pages past a file's end are left out natively.
  const typedRange = batchRange && pageRange.trim() ? parsePageRanges(pageRange) : null;
  const rangePicks = !batchRange ? selectedPages : inspectedDocument && typedRange?.ok ? pagesWithin(typedRange.value, inspectedDocument.pages.length) : [];
  const togglePage = (page: number) => {
    if (pagesPerPdf) {
      const current = shownPages;
      if (inputPath) setPicks({ ...picks, [inputPath]: current.includes(page) ? current.filter((selected) => selected !== page) : [...current, page].sort((left, right) => left - right) });
      return;
    }
    const nextPages = rangePicks.includes(page)
      ? rangePicks.filter((selected) => selected !== page)
      : [...rangePicks, page].sort((left, right) => left - right);
    setSelectedPages(nextPages);
    setPageRange(nextPages.length ? formatPageRange(nextPages) : "");
    setPageRangeFromPicks(nextPages.length > 0);
  };

  const choosePageRange = (value: string) => {
    setPageRange(value);
    setPageRangeFromPicks(false);
    if (!inspectedDocument || pagesPerPdf || batchRange) return;
    const parsed = parsePageRange(value, inspectedDocument.pages.length);
    setSelectedPages(Array.isArray(parsed) ? parsed : []);
  };
  const inspectionReady = inspection.kind === "ready" && inspection.path === inputPath && inspectedDocument !== null;
  const parsedSelection = inspectedDocument && !batchRange ? parsePageRange(pageRange, inspectedDocument.pages.length) : [];
  const selectionIssue = Array.isArray(parsedSelection) ? null : parsedSelection;
  const selectionCount = inspectedDocument ? rangePicks.length : 0;
  const needsPageSelection = activeUtility.capability.supportsPageSelection;
  const shownPages = !inspectedDocument ? [] : pagesPerPdf
    ? inspectedPicks ?? pagesPerPdf(options, pageRange, inspectedDocument.pages.length) ?? []
    : rangePicks;
  const emptyPicks = pagesPerPdf ? files.some((path) => picks[path]?.length === 0) : false;
  const canExport = files.length > 0 && activeUtility.capability.nativeAvailability === "available" && !tool.exportBlocked?.(options, files, pageRange)
    && (pagesPerPdf ? !emptyPicks : !tool.inspectsPdf || batchRange || inspectionReady);
  const pageSelectionBlocked = batchRange
    ? typedRange !== null && !typedRange.ok
    : needsPageSelection && !pagesPerPdf && (!inspectedDocument || selectionCount === 0 || selectionIssue !== null);
  const useRangeInstead = () => {
    if (!inputPath) return;
    const { [inputPath]: _dropped, ...rest } = picks;
    setPicks(rest);
  };
  const outputPreviewLimitation = activeUtility.capability.supportsPreview ? null : tool.outputPreview(options, selectedPages);
  const previewRegionLabel = outputPreviewLimitation ? "Source page selection" : "Conversion preview";
  const OptionsPanel = tool.OptionsPanel;
  const exportNote = tool.exportNote?.(files) ?? null;

  return (
    <Card className="workspace-control-panel py-0">
      <CardContent className="workspace-control-panel-content grid gap-4 p-4">
      {needsPageSelection && (
        <Card className="conversion-preview py-0" role="region" aria-label={previewRegionLabel}>
          <CardContent className="conversion-preview-content py-4">
          <div className="workspace-panel-intro">
            <p className="workspace-panel-label">{outputPreviewLimitation ? "Source page selection" : "Page selection"}</p>
            <p className="workspace-panel-copy">{outputPreviewLimitation ? "Choose exactly which source pages this conversion should include." : "Preview the rendered output and choose exactly what this conversion should include."}</p>
          </div>
          {inspectedDocument ? (
            <div className="conversion-page-grid">
              {inspectedDocument.pages.map((page) => (
                <Card className="conversion-page-card py-0" key={page.index}>
                  <CardContent className="grid gap-2 p-2">
                    <Label className="conversion-page-choice cursor-pointer flex-col items-stretch gap-2" htmlFor={`conversion-page-${page.index}`}>
                      <Checkbox
                        id={`conversion-page-${page.index}`}
                        aria-label={`Page ${page.index + 1}`}
                        checked={shownPages.includes(page.index)}
                        onCheckedChange={() => togglePage(page.index)}
                      />
                      {page.preview ? <img src={page.preview} alt={`Source page preview ${page.index + 1}`} /> : <span className="conversion-page-placeholder">Source preview unavailable</span>}
                      <span>Page {page.index + 1}</span>
                    </Label>
                  </CardContent>
                </Card>
              ))}
            </div>
          ) : (
            <p className="workspace-note">{inspection.kind === "error" && inspection.path === inputPath
              ? inspectError
              : batchRange && files.length > 0 ? "With one PDF in the list, its pages appear here as thumbnails to pick from." : "Select a PDF to load its page previews."}</p>
          )}
          {inspectedDocument && !pagesPerPdf && !batchRange && <p className="workspace-note">{selectionIssue?.message || selectedPageLabel(selectionCount)}</p>}
          {inspectedDocument && batchRange && typedRange?.ok !== false && (
            <>
              <p className="workspace-note" role="status">{pickStatus(rangePicks, inspectedDocument.pages.length, pageRange, pageRangeFromPicks)}</p>
              <p className="workspace-panel-copy">Click a page or press Space to pick it. Picked pages always run in page order. Picking nothing means every page.</p>
            </>
          )}
          {inspectedDocument && pagesPerPdf && (
            <div className="grid gap-2">
              <p className="workspace-note" role="status">
                {inspectedPicks
                  ? inspectedPicks.length
                    ? `${inspectedPicks.length} of ${inspectedDocument.pages.length} pages picked: ${listPages(inspectedPicks)}. These picks replace the range for this PDF only.`
                    : "No pages picked. Pick at least one page, or use the range instead."
                  : "Pick pages here to replace the range for this PDF only."}
              </p>
              <div className="flex flex-wrap gap-2">
                <Button variant="outline" size="sm" onClick={() => inputPath && setPicks({ ...picks, [inputPath]: inspectedDocument.pages.map((page) => page.index) })}>Pick all</Button>
                {inspectedPicks && <Button variant="ghost" size="sm" onClick={useRangeInstead}>Use the range instead</Button>}
              </div>
            </div>
          )}
          </CardContent>
        </Card>
      )}
      {outputPreviewLimitation && <Badge variant="outline" className="max-w-full whitespace-normal" role="status">{outputPreviewLimitation}</Badge>}
      {activeUtility.capability.nativeAvailability === "unavailable" && <Badge variant="outline" className="max-w-full whitespace-normal" role="alert">{activeUtility.title} is unavailable in this build.</Badge>}

      {workspaceForTool(activeUtility.id)?.id === "pdf-convert" && <div className="workspace-panel-intro">
        <div className="flex items-center justify-between gap-3">
          <p className="workspace-panel-label">Choose an output</p>
          <Badge variant="outline">On this device</Badge>
        </div>
        <p className="workspace-panel-copy">
          Keep one source surface and switch between page conversion, extraction,
          and document conversion outcomes.
        </p>
      </div>}
      {OptionsPanel && (
        <OptionsPanel
          options={options}
          setOptions={setOptions}
          files={files}
          selectedFileIndex={selectedFileIndex}
          inspectedDocument={inspectedDocument}
          selectedPages={selectedPages}
          pageRange={pageRange}
          choosePageRange={choosePageRange}
          pageRangeFromPicks={pageRangeFromPicks}
          picks={picks}
          pageCounts={pageCounts}
          loading={loading}
          onSelectTool={onSelectTool}
        />
      )}
      {destination}
      {tool.RunSummary && <tool.RunSummary options={options} files={files} pageRange={pageRange} picks={picks} pageCounts={pageCounts} />}
      {exportNote && <p className="workspace-note" role="status">{exportNote}</p>}
      <Button
        variant="default"
        disabled={loading || !canExport || pageSelectionBlocked}
        onClick={run}
        className="workspace-primary-action"
        aria-label={`Export ${activeUtility.title}`}
        {...shortcutTarget("run")}
      >
        {runLabel ?? `Export ${activeUtility.shortTitle}`}
        <ShortcutHint id="run" />
      </Button>
      </CardContent>
    </Card>
  );
};
