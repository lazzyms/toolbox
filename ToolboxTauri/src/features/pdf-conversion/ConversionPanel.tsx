import { useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { parsePageRange, selectedPageLabel } from "../../shared/workspaceValidation";
import type { PdfDocument } from "../pdf-editor";
import type { AtomicToolId, ToolDefinition } from "../../contracts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { ShortcutHint } from "../../components/ShortcutHint";
import { shortcutTarget } from "../../shortcuts";
import { formatPageRange, type ConversionTool } from "./tool";

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
  onSelectTool?: (id: AtomicToolId) => void;
};

export const ConversionPanel = ({
  destination,
  runLabel,
  activeUtility,
  tool,
  options,
  setOptions,
  files,
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
  onSelectTool,
}: ConversionPanelProps) => {
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
  }, [tool, inputPath, inputPolicyIssue, setDocument, setInspectError, setPageRange, setSelectedPages]);

  const togglePage = (page: number) => {
    const nextPages = selectedPages.includes(page)
      ? selectedPages.filter((selected) => selected !== page)
      : [...selectedPages, page].sort((left, right) => left - right);
    setSelectedPages(nextPages);
    setPageRange(nextPages.length ? formatPageRange(nextPages) : "");
  };

  const inspectedDocument = inspection.kind === "ready" && inspection.path === inputPath ? document : null;
  const choosePageRange = (value: string) => {
    setPageRange(value);
    if (!inspectedDocument) return;
    const parsed = parsePageRange(value, inspectedDocument.pages.length);
    setSelectedPages(Array.isArray(parsed) ? parsed : []);
  };
  const inspectionReady = inspection.kind === "ready" && inspection.path === inputPath && inspectedDocument !== null;
  const parsedSelection = inspectedDocument ? parsePageRange(pageRange, inspectedDocument.pages.length) : [];
  const selectionIssue = Array.isArray(parsedSelection) ? null : parsedSelection;
  const selectionCount = inspectedDocument ? selectedPages.length : 0;
  const needsPageSelection = activeUtility.capability.supportsPageSelection;
  const canExport = files.length > 0 && activeUtility.capability.nativeAvailability === "available" && !tool.exportBlocked?.(options, files, pageRange)
    && (!tool.inspectsPdf || inspectionReady);
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
      {OptionsPanel && (
        <OptionsPanel
          options={options}
          setOptions={setOptions}
          files={files}
          inspectedDocument={inspectedDocument}
          selectedPages={selectedPages}
          pageRange={pageRange}
          choosePageRange={choosePageRange}
          onSelectTool={onSelectTool}
        />
      )}
      {destination}
      {exportNote && <p className="workspace-note" role="status">{exportNote}</p>}
      <Button
        variant="default"
        disabled={loading || !canExport || (needsPageSelection && (!inspectedDocument || selectionCount === 0 || selectionIssue !== null))}
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
