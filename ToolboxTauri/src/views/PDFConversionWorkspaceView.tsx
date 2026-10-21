import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { parsePageRange, selectedPageLabel } from "../shared/workspaceValidation";
import { displayedBoxToPdfRect, type DisplayedBox, type PdfDocument } from "../features/pdf-editor";
import type { AtomicToolId, ToolDefinition, ToolResult } from "../contracts";
import { failedOutcome } from "../jobOutcome";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";

const nonInspectingConversionIds = new Set(["images-to-pdf", "pdf-merge", "pdf-split", "pdf-repair", "pdf-compare"]);
const outputPreviewLimitations: Partial<Record<AtomicToolId, string>> = {
  "images-to-pdf": "Output preview unavailable. Export combines the selected images into one PDF in file order.",
  "pdf-merge": "Output preview unavailable. Export merges the selected PDFs in file order.",
  "pdf-split": "Output preview unavailable. Export creates separate PDF files using the selected split mode.",
  "pdf-to-text": "Output preview unavailable. Export creates a text file from the selected pages.",
  "pdf-image-extract": "Output preview unavailable. Export extracts embedded JPEG images from the selected pages.",
  "pdf-extract-pages": "Output preview unavailable. Export creates a PDF containing the selected pages.",
  "pdf-ocr": "Output preview unavailable. OCR is unavailable in this build.",
  "pdf-rotate": "Output preview unavailable. Export saves a copy with the selected pages turned.",
  "pdf-repair": "Output preview unavailable. Export rebuilds each PDF and saves a fixed copy next to it.",
  "pdf-redact": "Output preview unavailable. Export saves a copy with the boxed area removed for good. Redacted pages become images.",
  "pdf-compare": "Output preview unavailable. Export creates a text file listing every line that changed, page by page.",
};
type PdfInspectionState =
  | { kind: "idle" }
  | { kind: "pending"; path: string }
  | { kind: "ready"; path: string }
  | { kind: "error"; path: string };

const formatPageRange = (pages: number[]) => {
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

const pdfToImagesOutputSummary = (selectedPages: number[], dpi: string, format: string) => {
  const pageSummary = selectedPages.length
    ? `${selectedPages.length === 1 ? "page" : "pages"} ${formatPageRange(selectedPages)}`
    : "the selected pages";
  const formatLabel = format === "jpg" ? "JPEG" : format.toUpperCase();
  return `Output preview unavailable. Export renders ${pageSummary} at ${dpi} DPI as ${formatLabel} image files.`;
};

export const PDFConversionWorkspaceView = ({ utility: activeUtility, initialPaths = [] }: { utility: ToolDefinition; initialPaths?: readonly string[] }) => {
  const [dpi, setDpi] = useState("150");
  const [format, setFormat] = useState("jpg");
  const [pageRange, setPageRange] = useState("");
  const [splitMode, setSplitMode] = useState<"pages" | "ranges" | "chunks">("pages");
  const [chunkSize, setChunkSize] = useState(2);
  const [degrees, setDegrees] = useState(90);
  const [redactBox, setRedactBox] = useState<DisplayedBox | null>(null);
  const [document, setDocument] = useState<PdfDocument | null>(null);
  const [selectedPages, setSelectedPages] = useState<number[]>([]);
  const [inspectError, setInspectError] = useState("");

  return (
    <ToolScaffold
      key={initialPaths.join('\0') || 'no-initial-pdf'}
      variant="workspace"
      sessionKey="pdf-convert"
      utility={activeUtility}
      initialPaths={initialPaths}
      onRun={(paths, outputLocation, { outputPath, runId }) => {
        const pages = document ? selectedPages : undefined;
        const selectedPageRange = pages?.length ? formatPageRange(pages) : pageRange || null;
        if (activeUtility.id === "images-to-pdf") {
          return invoke<ToolResult>("images_to_pdf", { runId,
            request: { paths, outputLocation, outputPath },
          });
        }
        if (activeUtility.id === "pdf-merge") {
          return invoke<ToolResult>("merge_pdfs", { runId,
            request: { paths, outputLocation, outputPath },
          });
        }
        if (activeUtility.id === "pdf-split") {
          return invoke<ToolResult>("split_pdf", { runId,
            request: {
              paths,
              pages: [],
              pageRanges: splitMode === "ranges" ? pageRange || null : null,
              splitMode,
              chunkSize: splitMode === "chunks" ? chunkSize : null,
              outputLocation,
            },
          });
        }
        if (activeUtility.id === "pdf-extract-pages") {
          return invoke<ToolResult>("extract_pdf_pages", { runId,
            request: {
              paths,
              pages: selectedPages,
              pageRanges: pageRange || null,
              outputLocation,
            },
          });
        }
        if (activeUtility.id === "pdf-rotate") {
          const somePages = document && selectedPages.length < document.pages.length;
          return invoke<ToolResult>("rotate_pdf", { runId,
            request: { paths, degrees, pages: somePages ? selectedPages : null, outputLocation },
          });
        }
        if (activeUtility.id === "pdf-repair") {
          return invoke<ToolResult>("repair_pdf", { runId,
            request: { paths, outputLocation },
          });
        }
        if (activeUtility.id === "pdf-compare") {
          return invoke<ToolResult>("compare_pdfs", { runId,
            request: { paths, outputLocation, outputPath },
          });
        }
        if (activeUtility.id === "pdf-redact") {
          const drawnOn = document?.pages[selectedPages[0] ?? 0];
          if (!redactBox || !drawnOn) {
            return Promise.resolve([failedOutcome(paths[0] ?? "", "invalidInput", "Draw a box over what you want to hide first.")]);
          }
          const allPages = selectedPages.length === document.pages.length;
          return invoke<ToolResult>("redact_pdf", { runId,
            request: {
              paths,
              rectangle: displayedBoxToPdfRect(redactBox, drawnOn),
              scope: allPages ? "all" : { selected: { pages: selectedPages } },
              outputLocation,
            },
          });
        }
        if (activeUtility.id === "pdf-to-images") {
          return invoke<ToolResult>("pdf_to_images", { runId,
            request: {
              paths,
              dpi: Number(dpi),
              format,
              pageRange: selectedPageRange,
              pages: pages ?? null,
              outputLocation,
            },
          });
        }
        if (activeUtility.id === "pdf-to-text") {
          return invoke<ToolResult>("pdf_to_text", { runId,
            request: { paths, pages: pages ?? null, outputLocation },
          });
        }
        if (activeUtility.id === "pdf-ocr") {
          return invoke<ToolResult>("ocr_pdf", { runId,
            request: { paths, pages: pages ?? null, outputLocation },
          });
        }
        return invoke<ToolResult>("extract_pdf_images", { runId,
          request: { paths, pages: pages ?? null, outputLocation },
        });
      }}
    >
      {({ files, run, loading, destination, runLabel }) => (
        <ConversionControls
          destination={destination}
          runLabel={runLabel}
          activeUtility={activeUtility}
          files={files}
          run={run}
          loading={loading}
          dpi={dpi}
          setDpi={setDpi}
          format={format}
          setFormat={setFormat}
          pageRange={pageRange}
          setPageRange={setPageRange}
          splitMode={splitMode}
          setSplitMode={setSplitMode}
          chunkSize={chunkSize}
          setChunkSize={setChunkSize}
          degrees={degrees}
          setDegrees={setDegrees}
          redactBox={redactBox}
          setRedactBox={setRedactBox}
          document={document}
          setDocument={setDocument}
          selectedPages={selectedPages}
          setSelectedPages={setSelectedPages}
          inspectError={inspectError}
          setInspectError={setInspectError}
        />
      )}
    </ToolScaffold>
  );
};

type ConversionControlsProps = {
  destination: React.ReactNode;
  runLabel: string | null;
  activeUtility: ToolDefinition;
  files: string[];
  run: () => Promise<void>;
  loading: boolean;
  dpi: string;
  setDpi: (value: string) => void;
  format: string;
  setFormat: (value: string) => void;
  pageRange: string;
  setPageRange: (value: string) => void;
  splitMode: "pages" | "ranges" | "chunks";
  setSplitMode: (value: "pages" | "ranges" | "chunks") => void;
  chunkSize: number;
  setChunkSize: (value: number) => void;
  degrees: number;
  setDegrees: (value: number) => void;
  redactBox: DisplayedBox | null;
  setRedactBox: (value: DisplayedBox | null) => void;
  document: PdfDocument | null;
  setDocument: (value: PdfDocument | null) => void;
  selectedPages: number[];
  setSelectedPages: (value: number[]) => void;
  inspectError: string;
  setInspectError: (value: string) => void;
};

const ConversionControls = ({
  destination,
  runLabel,
  activeUtility,
  files,
  run,
  loading,
  dpi,
  setDpi,
  format,
  setFormat,
  pageRange,
  setPageRange,
  splitMode,
  setSplitMode,
  chunkSize,
  setChunkSize,
  degrees,
  setDegrees,
  redactBox,
  setRedactBox,
  document,
  setDocument,
  selectedPages,
  setSelectedPages,
  inspectError,
  setInspectError,
}: ConversionControlsProps) => {
  const inputPath = files[0];
  const [inspection, setInspection] = useState<PdfInspectionState>({ kind: "idle" });
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
    let current = true;
    setDocument(null);
    setSelectedPages([]);
    setPageRange("");
    setInspectError("");
    setRedactBox(null);
    if (!inputPath || inputPolicyIssue) {
      setInspection({ kind: "idle" });
      return;
    }
    if (nonInspectingConversionIds.has(activeUtility.id)) {
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
        // Blacking out every page by accident is easy to miss, so redaction starts on the first page only.
        setSelectedPages(activeUtility.id === "pdf-redact" ? value.pages.slice(0, 1).map((page) => page.index) : value.pages.map((page) => page.index));
        setPageRange("");
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
  }, [activeUtility.id, inputPath, inputPolicyIssue, setDocument, setInspectError, setPageRange, setRedactBox, setSelectedPages]);

  const togglePage = (page: number) => {
    const nextPages = selectedPages.includes(page)
      ? selectedPages.filter((selected) => selected !== page)
      : [...selectedPages, page].sort((left, right) => left - right);
    setSelectedPages(nextPages);
    setPageRange(nextPages.length ? formatPageRange(nextPages) : "");
  };

  const inspectedDocument = inspection.kind === "ready" && inspection.path === inputPath ? document : null;
  const inspectionReady = inspection.kind === "ready" && inspection.path === inputPath && inspectedDocument !== null;
  const parsedSelection = inspectedDocument ? parsePageRange(pageRange, inspectedDocument.pages.length) : [];
  const selectionIssue = Array.isArray(parsedSelection) ? null : parsedSelection;
  const selectionCount = inspectedDocument ? selectedPages.length : 0;
  const needsPageSelection = activeUtility.capability.supportsPageSelection;
  const needsMultipleInputs = activeUtility.id === "pdf-merge";
  const compareIssue = activeUtility.id === "pdf-compare" && files.length !== 2;
  const redactPage = activeUtility.id === "pdf-redact" ? inspectedDocument?.pages[selectedPages[0] ?? -1] : undefined;
  const rangeSelectionIssue = activeUtility.id === "pdf-split" && splitMode === "ranges" && !pageRange.trim()
    ? "Enter at least one page range to split by ranges."
    : null;
  const canExport = files.length > 0 && activeUtility.capability.nativeAvailability === "available" && (!needsMultipleInputs || files.length > 1) && rangeSelectionIssue === null && !compareIssue
    && (nonInspectingConversionIds.has(activeUtility.id) || inspectionReady);
  const outputPreviewLimitation = activeUtility.id === "pdf-to-images"
    ? pdfToImagesOutputSummary(selectedPages, dpi, format)
    : activeUtility.capability.supportsPreview
      ? null
      : outputPreviewLimitations[activeUtility.id] ?? "Output preview unavailable for this conversion.";
  const previewRegionLabel = outputPreviewLimitation ? "Source page selection" : "Conversion preview";

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
                        checked={selectedPages.includes(page.index)}
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
            <p className="workspace-note">{inspection.kind === "error" && inspection.path === inputPath ? inspectError : "Select a PDF to load its page previews."}</p>
          )}
          {inspectedDocument && <p className="workspace-note">{selectionIssue?.message || selectedPageLabel(selectionCount)}</p>}
          </CardContent>
        </Card>
      )}
      {outputPreviewLimitation && <Badge variant="outline" className="max-w-full whitespace-normal" role="status">{outputPreviewLimitation}</Badge>}
      {activeUtility.capability.nativeAvailability === "unavailable" && <Badge variant="outline" className="max-w-full whitespace-normal" role="alert">{activeUtility.title} is unavailable in this build.</Badge>}

      <div className="workspace-panel-intro">
        <div className="flex items-center justify-between gap-3">
          <p className="workspace-panel-label">Choose an output</p>
          <Badge variant="outline">On this device</Badge>
        </div>
        <p className="workspace-panel-copy">
          Keep one source surface and switch between page conversion, extraction,
          document assembly, and splitting outcomes.
        </p>
      </div>
      {activeUtility.id === "pdf-to-images" && (
        <div className="workspace-field-grid">
          <div className="workspace-field">
            <Label htmlFor="render-dpi">Resolution</Label>
            <NativeSelect id="render-dpi" aria-label="Render DPI" value={dpi} onChange={(event) => setDpi(event.target.value)}>
              <option value="72">72 DPI</option>
              <option value="150">150 DPI</option>
              <option value="300">300 DPI</option>
            </NativeSelect>
          </div>
          <div className="workspace-field">
            <Label htmlFor="image-format">Format</Label>
            <NativeSelect id="image-format" aria-label="Output format" value={format} onChange={(event) => setFormat(event.target.value)}>
              <option value="jpg">JPEG</option>
              <option value="png">PNG</option>
            </NativeSelect>
          </div>
          <div className="workspace-field workspace-field-wide">
            <Label htmlFor="image-page-range">Pages</Label>
            <Input
              id="image-page-range"
              aria-label="Page range"
              value={pageRange}
              onChange={(event) => {
                const value = event.target.value;
                setPageRange(value);
                if (!inspectedDocument) return;
                const parsed = parsePageRange(value, inspectedDocument.pages.length);
                setSelectedPages(Array.isArray(parsed) ? parsed : []);
              }}
              placeholder="All pages, or 1-3"
            />
          </div>
        </div>
      )}
      {activeUtility.id === "pdf-to-text" && (
        <p className="workspace-note">
          Selectable text is extracted in page order. Scanned PDFs are reported as needing OCR.
        </p>
      )}
      {activeUtility.id === "pdf-image-extract" && (
        <p className="workspace-note">
          Embedded JPEG images are saved at their original resolution when the PDF contains them.
        </p>
      )}
      {activeUtility.id === "images-to-pdf" && (
        <p className="workspace-note">Images are combined in the order they appear in the selected list.</p>
      )}
      {activeUtility.id === "pdf-merge" && (
        <p className="workspace-note">Add at least two PDFs. The order in the source list becomes the page order in the merged copy.</p>
      )}
      {activeUtility.id === "pdf-split" && (
        <div className="workspace-field-grid">
          <div className="workspace-field">
            <Label htmlFor="split-mode">Split mode</Label>
            <NativeSelect id="split-mode" aria-label="Split mode" value={splitMode} onChange={(event) => setSplitMode(event.target.value as typeof splitMode)}>
              <option value="pages">Every page</option>
              <option value="ranges">Ranges</option>
              <option value="chunks">Fixed-size chunks</option>
            </NativeSelect>
          </div>
          {splitMode === "ranges" && (
            <div className="workspace-field">
              <Label htmlFor="split-ranges">Ranges</Label>
              <Input id="split-ranges" aria-label="Page ranges" value={pageRange} onChange={(event) => setPageRange(event.target.value)} placeholder="1-3, 4-8, 9-" />
            </div>
          )}
          {splitMode === "chunks" && (
            <div className="workspace-field">
              <Label htmlFor="split-chunk-size">Pages per file</Label>
              <Input id="split-chunk-size" aria-label="Pages per file" type="number" min="1" value={chunkSize} onChange={(event) => setChunkSize(Number(event.target.value))} />
            </div>
          )}
        </div>
      )}
      {rangeSelectionIssue && <p className="workspace-note" role="alert">{rangeSelectionIssue}</p>}
      {activeUtility.id === "pdf-extract-pages" && (
        <div className="workspace-field">
          <Label htmlFor="extract-pages-range">Pages or ranges</Label>
          <Input id="extract-pages-range" aria-label="Page numbers or ranges" value={pageRange} onChange={(event) => { const value = event.target.value; setPageRange(value); if (!inspectedDocument) return; const parsed = parsePageRange(value, inspectedDocument.pages.length); setSelectedPages(Array.isArray(parsed) ? parsed : []); }} placeholder="1-3, 7" />
        </div>
      )}
      {activeUtility.id === "pdf-rotate" && (
        <div className="workspace-field">
          <Label htmlFor="rotate-direction">Direction</Label>
          <NativeSelect id="rotate-direction" aria-label="Rotation direction" value={String(degrees)} onChange={(event) => setDegrees(Number(event.target.value))}>
            <option value="90">Clockwise 90°</option>
            <option value="270">Counterclockwise 90°</option>
            <option value="180">180°</option>
          </NativeSelect>
        </div>
      )}
      {activeUtility.id === "pdf-repair" && (
        <p className="workspace-note">Toolbox rebuilds each PDF and saves a fixed copy next to it.</p>
      )}
      {activeUtility.id === "pdf-compare" && (
        files.length === 2 ? (
          <div className="grid gap-2" aria-live="polite">
            <p className="workspace-note">Original: <strong>{fileName(files[0])}</strong></p>
            <p className="workspace-note">New version: <strong>{fileName(files[1])}</strong></p>
            <p className="workspace-note">Drag the files in the list to swap them.</p>
          </div>
        ) : (
          <p className="workspace-note" role="status">Choose exactly two PDFs: the original and the new version.</p>
        )
      )}
      {activeUtility.id === "pdf-redact" && redactPage?.preview && (
        <RedactionArea page={redactPage.index} preview={redactPage.preview} aspectRatio={redactPage.width / redactPage.height} box={redactBox} onChange={setRedactBox} />
      )}
      {destination}
      <Button
        variant="default"
        disabled={loading || !canExport || (needsPageSelection && (!inspectedDocument || selectionCount === 0 || selectionIssue !== null)) || (activeUtility.id === "pdf-redact" && !redactBox)}
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

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

const clampFraction = (value: number) => Math.min(Math.max(value, 0), 1);

const RedactionArea = ({ page, preview, aspectRatio, box, onChange }: { page: number; preview: string; aspectRatio: number; box: DisplayedBox | null; onChange: (box: DisplayedBox | null) => void }) => {
  const [start, setStart] = useState<{ x: number; y: number } | null>(null);
  const fractionAt = (event: React.PointerEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    return { x: clampFraction((event.clientX - bounds.left) / bounds.width), y: clampFraction((event.clientY - bounds.top) / bounds.height) };
  };
  const boxFrom = (from: { x: number; y: number }, to: { x: number; y: number }): DisplayedBox => ({
    left: Math.min(from.x, to.x), top: Math.min(from.y, to.y), right: Math.max(from.x, to.x), bottom: Math.max(from.y, to.y),
  });
  return (
    <div className="grid gap-2">
      <p className="workspace-note">Drag over the details you want to hide on page {page + 1}. The same area is removed on every checked page.</p>
      <div
        role="region"
        aria-label="Redaction area"
        className="relative cursor-crosshair touch-none select-none border"
        style={{ aspectRatio }}
        onPointerDown={(event) => { event.currentTarget.setPointerCapture(event.pointerId); const point = fractionAt(event); setStart(point); onChange(null); }}
        onPointerMove={(event) => { if (start) onChange(boxFrom(start, fractionAt(event))); }}
        onPointerUp={(event) => {
          if (!start) return;
          const next = boxFrom(start, fractionAt(event));
          setStart(null);
          onChange(next.right - next.left > 0.005 && next.bottom - next.top > 0.005 ? next : null);
        }}
        onPointerCancel={() => { setStart(null); onChange(null); }}
      >
        <img src={preview} alt={`Page ${page + 1} to redact`} draggable={false} className="block size-full" />
        {box && (
          <div
            aria-label="Redaction box"
            className="absolute bg-black/80"
            style={{ left: `${box.left * 100}%`, top: `${box.top * 100}%`, width: `${(box.right - box.left) * 100}%`, height: `${(box.bottom - box.top) * 100}%` }}
          />
        )}
      </div>
      {box && <Button variant="outline" size="sm" className="justify-self-start" onClick={() => onChange(null)}>Clear box</Button>}
    </div>
  );
};
