import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { UtilityRegistry } from "../registry";
import type { ToolDefinition, ToolResult } from "../contracts";

const conversionIds = new Set([
  "pdf-to-images",
  "pdf-to-text",
  "pdf-image-extract",
  "images-to-pdf",
]);

export const PDFConversionWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const [dpi, setDpi] = useState("150");
  const [format, setFormat] = useState("jpg");
  const [pageRange, setPageRange] = useState("");
  const activeUtility = conversionIds.has(utility.id)
    ? utility
    : UtilityRegistry.find((item) => item.id === "pdf-to-images") ?? utility;

  return (
    <ToolScaffold
      utility={activeUtility}
      onRun={(paths) => {
        if (activeUtility.id === "images-to-pdf") {
          return invoke<ToolResult>("images_to_pdf", {
            request: { paths, outputLocation: "alongsideInput" },
          });
        }
        if (activeUtility.id === "pdf-to-images") {
          return invoke<ToolResult>("pdf_to_images", {
            request: {
              paths,
              dpi: Number(dpi),
              format,
              pageRange: pageRange || null,
              outputLocation: "alongsideInput",
            },
          });
        }
        if (activeUtility.id === "pdf-to-text") {
          return invoke<ToolResult>("pdf_to_text", {
            request: { paths, outputLocation: "alongsideInput" },
          });
        }
        return invoke<ToolResult>("extract_pdf_images", {
          request: { paths, outputLocation: "alongsideInput" },
        });
      }}
    >
      {({ files, run, loading }) => (
        <div className="workspace-control-panel">
          <div>
            <p className="workspace-panel-label">Choose an output</p>
            <p className="workspace-panel-copy">
              Keep the same input selection and switch between rendered pages, selectable text,
              embedded images, or a new PDF from images.
            </p>
          </div>
          {activeUtility.id === "pdf-to-images" && (
            <div className="workspace-field-grid">
              <label className="workspace-field">
                <span>Resolution</span>
                <select aria-label="Render DPI" value={dpi} onChange={(event) => setDpi(event.target.value)}>
                  <option value="72">72 DPI</option>
                  <option value="150">150 DPI</option>
                  <option value="300">300 DPI</option>
                </select>
              </label>
              <label className="workspace-field">
                <span>Format</span>
                <select aria-label="Image format" value={format} onChange={(event) => setFormat(event.target.value)}>
                  <option value="jpg">JPEG</option>
                  <option value="png">PNG</option>
                </select>
              </label>
              <label className="workspace-field workspace-field-wide">
                <span>Pages</span>
                <input
                  aria-label="Page range"
                  value={pageRange}
                  onChange={(event) => setPageRange(event.target.value)}
                  placeholder="All pages, or 1-3"
                />
              </label>
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
          <button
            type="button"
            disabled={loading || files.length === 0}
            onClick={run}
            className="workspace-primary-action"
          >
            {activeUtility.shortTitle}
          </button>
        </div>
      )}
    </ToolScaffold>
  );
};
