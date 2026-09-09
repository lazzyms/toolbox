import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { ToolScaffold } from "../components/ToolScaffold";
import { UtilityRegistry } from "../registry";
import type { ToolDefinition, ToolResult } from "../contracts";
import { PdfEditor } from "../features/pdf-editor";
import type {
  CropPdfRequest,
  OrganizePdfRequest,
  PdfDocument,
  PdfEditorState,
  PdfRect,
  SignPdfRequest,
} from "../features/pdf-editor";

const pdfEditorActionIds = [
  "pdf-edit",
  "pdf-crop",
  "pdf-watermark",
  "pdf-sign",
  "pdf-page-numbers",
  "pdf-remove-pages",
  "pdf-organize",
  "pdf-merge",
  "pdf-split",
  "pdf-extract-pages",
  "pdf-compress",
] as const;

const pdfEditorIds = new Set<string>(pdfEditorActionIds);
const editModes = ["text", "note", "highlight", "shape"] as const;

type EditMode = (typeof editModes)[number];
type OrganizeMode = "arrange" | "add-pages";
type AddPagePosition = "before" | "after" | "end";

const createInitialState = (document?: PdfDocument): PdfEditorState => ({
  currentPage: 0,
  selectedPages: [],
  pageOrder: document?.pages.map((page) => page.index) ?? [],
  deletedPages: [],
  rotatePages: [],
  scope: { kind: "all" },
  layout: "vertical",
  density: "comfortable",
  error: null,
});

const pageList = (value: string) =>
  value
    .split(",")
    .map((page) => Number(page.trim()) - 1)
    .filter((page) => Number.isInteger(page) && page >= 0);

const pageScope = (state: PdfEditorState) =>
  state.scope.kind === "all" ? "all" : { selected: { pages: state.scope.pages } };

const fullPageRect = (document: PdfDocument | null, pageIndex: number): PdfRect | null => {
  const page = document?.pages[pageIndex];
  return page
    ? { x: page.x ?? 0, y: page.y ?? 0, width: page.width, height: page.height }
    : null;
};

const invalidResult = (paths: string[], message: string): ToolResult => [
  {
    inputPath: paths[0] ?? "",
    outputPaths: [],
    detail: "",
    failure: { kind: "invalidInput", message },
  },
];

export const PDFEditorWorkspaceView = ({ utility }: { utility: ToolDefinition }) => {
  const [document, setDocument] = useState<PdfDocument | null>(null);
  const [state, setState] = useState<PdfEditorState>(createInitialState);
  const [rectangle, setRectangle] = useState<PdfRect | null>(null);
  const [editMode, setEditMode] = useState<EditMode>("text");
  const [text, setText] = useState("");
  const [signaturePath, setSignaturePath] = useState<string | null>(null);
  const [watermarkText, setWatermarkText] = useState("Toolbox");
  const [watermarkOpacity, setWatermarkOpacity] = useState(70);
  const [watermarkPosition, setWatermarkPosition] = useState("center");
  const [watermarkLogoPath, setWatermarkLogoPath] = useState<string | null>(null);
  const [pages, setPages] = useState("");
  const [startNumber, setStartNumber] = useState(1);
  const [fontSize, setFontSize] = useState(12);
  const [splitMode, setSplitMode] = useState<"pages" | "ranges" | "chunks">("pages");
  const [chunkSize, setChunkSize] = useState(2);
  const [quality, setQuality] = useState(80);
  const [organizeMode, setOrganizeMode] = useState<OrganizeMode>("arrange");
  const [addPagePosition, setAddPagePosition] = useState<AddPagePosition>("after");
  const [addPageCount, setAddPageCount] = useState(1);
  const activeUtility = pdfEditorIds.has(utility.id)
    ? utility
    : UtilityRegistry.find((item) => item.id === "pdf-edit") ?? utility;

  return (
    <ToolScaffold
      utility={activeUtility}
      onRun={(paths) => {
        const scope = pageScope(state);
        const selectedRect = rectangle ?? fullPageRect(document, state.currentPage);

        if (activeUtility.id === "pdf-edit") {
          if (editMode !== "shape" && !text.trim()) {
            return Promise.resolve(invalidResult(paths, "Text is required for this edit mode."));
          }
          return invoke<ToolResult>("edit_pdf", {
            request: {
              paths,
              mode: editMode,
              text,
              pages: state.scope.kind === "all" ? null : state.scope.pages,
              rectangle: selectedRect ?? { x: 72, y: 650, width: 240, height: 72 },
              outputLocation: "alongsideInput",
            },
          });
        }
        if (activeUtility.id === "pdf-crop") {
          return selectedRect
            ? invoke<ToolResult>("crop_pdf", {
                request: { paths, rectangle: selectedRect, scope, outputLocation: "alongsideInput" } satisfies CropPdfRequest,
              })
            : Promise.resolve(invalidResult(paths, "Draw a crop rectangle first."));
        }
        if (activeUtility.id === "pdf-watermark") {
          return invoke<ToolResult>("watermark_pdf", {
            request: {
              paths,
              text: watermarkText,
              opacity: watermarkOpacity,
              position: watermarkPosition,
              logoPath: watermarkLogoPath,
              pages: pages ? pageList(pages) : null,
              outputLocation: "alongsideInput",
            },
          });
        }
        if (activeUtility.id === "pdf-sign") {
          return selectedRect
            ? invoke<ToolResult>("sign_pdf", {
                request: {
                  paths,
                  page: state.currentPage,
                  text: text || "Signature",
                  signaturePath,
                  rectangle: selectedRect,
                  scope,
                  outputLocation: "alongsideInput",
                } satisfies SignPdfRequest,
              })
            : Promise.resolve(invalidResult(paths, "Select a PDF page for the signature."));
        }
        if (activeUtility.id === "pdf-page-numbers") {
          return invoke<ToolResult>("add_page_numbers", {
            request: {
              paths,
              text: "",
              opacity: 100,
              position: watermarkPosition,
              logoPath: null,
              pages: pages ? pageList(pages) : null,
              startNumber,
              fontSize,
              outputLocation: "alongsideInput",
            },
          });
        }
        if (activeUtility.id === "pdf-remove-pages") {
          return state.deletedPages.length > 0
            ? invoke<ToolResult>("remove_pdf_pages", {
                request: { paths: [paths[0]], pages: state.deletedPages, pageRanges: null, outputLocation: "alongsideInput" },
              })
            : Promise.resolve(invalidResult(paths, "Delete at least one page in the editor first."));
        }
        if (activeUtility.id === "pdf-organize") {
          if (organizeMode === "add-pages") {
            return invoke<ToolResult>("add_pdf_pages", {
              request: {
                paths: [paths[0]],
                page: state.currentPage,
                position: addPagePosition,
                count: addPageCount,
                outputLocation: "alongsideInput",
              },
            });
          }
          return invoke<ToolResult>("organize_pdf", {
            request: {
              paths: [paths[0]],
              pageOrder: state.pageOrder,
              deletePages: state.deletedPages,
              rotatePages: state.rotatePages,
              scope,
              outputLocation: "alongsideInput",
            } satisfies OrganizePdfRequest,
          });
        }
        if (activeUtility.id === "pdf-merge") {
          return invoke<ToolResult>("merge_pdfs", { request: { paths, outputLocation: "alongsideInput" } });
        }
        if (activeUtility.id === "pdf-split") {
          return invoke<ToolResult>("split_pdf", {
            request: {
              paths: [paths[0]],
              pages: [],
              pageRanges: splitMode === "ranges" ? pages || null : null,
              splitMode,
              chunkSize: splitMode === "chunks" ? chunkSize : null,
              outputLocation: "alongsideInput",
            },
          });
        }
        if (activeUtility.id === "pdf-extract-pages") {
          return invoke<ToolResult>("extract_pdf_pages", {
            request: {
              paths: [paths[0]],
              pages: state.selectedPages,
              pageRanges: pages || null,
              outputLocation: "alongsideInput",
            },
          });
        }
        return invoke<ToolResult>("compress_pdf", {
          request: { paths: [paths[0]], quality, outputLocation: "alongsideInput" },
        });
      }}
    >
      {(props) => (
        <PDFEditorContent
          {...props}
          utility={activeUtility}
          document={document}
          setDocument={setDocument}
          state={state}
          setState={setState}
          rectangle={rectangle}
          setRectangle={setRectangle}
          editMode={editMode}
          setEditMode={setEditMode}
          text={text}
          setText={setText}
          signaturePath={signaturePath}
          setSignaturePath={setSignaturePath}
          watermarkText={watermarkText}
          setWatermarkText={setWatermarkText}
          watermarkOpacity={watermarkOpacity}
          setWatermarkOpacity={setWatermarkOpacity}
          watermarkPosition={watermarkPosition}
          setWatermarkPosition={setWatermarkPosition}
          watermarkLogoPath={watermarkLogoPath}
          setWatermarkLogoPath={setWatermarkLogoPath}
          pages={pages}
          setPages={setPages}
          startNumber={startNumber}
          setStartNumber={setStartNumber}
          fontSize={fontSize}
          setFontSize={setFontSize}
          splitMode={splitMode}
          setSplitMode={setSplitMode}
          chunkSize={chunkSize}
          setChunkSize={setChunkSize}
          quality={quality}
          setQuality={setQuality}
          organizeMode={organizeMode}
          setOrganizeMode={setOrganizeMode}
          addPagePosition={addPagePosition}
          setAddPagePosition={setAddPagePosition}
          addPageCount={addPageCount}
          setAddPageCount={setAddPageCount}
        />
      )}
    </ToolScaffold>
  );
};

interface PDFEditorContentProps {
  files: string[];
  run: () => Promise<void>;
  loading: boolean;
  utility: ToolDefinition;
  document: PdfDocument | null;
  setDocument: (document: PdfDocument | null) => void;
  state: PdfEditorState;
  setState: (state: PdfEditorState) => void;
  rectangle: PdfRect | null;
  setRectangle: (rectangle: PdfRect | null) => void;
  editMode: EditMode;
  setEditMode: (mode: EditMode) => void;
  text: string;
  setText: (text: string) => void;
  signaturePath: string | null;
  setSignaturePath: (path: string | null) => void;
  watermarkText: string;
  setWatermarkText: (text: string) => void;
  watermarkOpacity: number;
  setWatermarkOpacity: (value: number) => void;
  watermarkPosition: string;
  setWatermarkPosition: (value: string) => void;
  watermarkLogoPath: string | null;
  setWatermarkLogoPath: (path: string | null) => void;
  pages: string;
  setPages: (pages: string) => void;
  startNumber: number;
  setStartNumber: (value: number) => void;
  fontSize: number;
  setFontSize: (value: number) => void;
  splitMode: "pages" | "ranges" | "chunks";
  setSplitMode: (value: "pages" | "ranges" | "chunks") => void;
  chunkSize: number;
  setChunkSize: (value: number) => void;
  quality: number;
  setQuality: (value: number) => void;
  organizeMode: OrganizeMode;
  setOrganizeMode: (value: OrganizeMode) => void;
  addPagePosition: AddPagePosition;
  setAddPagePosition: (value: AddPagePosition) => void;
  addPageCount: number;
  setAddPageCount: (value: number) => void;
}

const PDFEditorContent = ({
  files,
  run,
  loading,
  utility,
  document,
  setDocument,
  state,
  setState,
  rectangle,
  setRectangle,
  editMode,
  setEditMode,
  text,
  setText,
  signaturePath,
  setSignaturePath,
  watermarkText,
  setWatermarkText,
  watermarkOpacity,
  setWatermarkOpacity,
  watermarkPosition,
  setWatermarkPosition,
  watermarkLogoPath,
  setWatermarkLogoPath,
  pages,
  setPages,
  startNumber,
  setStartNumber,
  fontSize,
  setFontSize,
  splitMode,
  setSplitMode,
  chunkSize,
  setChunkSize,
  quality,
  setQuality,
  organizeMode,
  setOrganizeMode,
  addPagePosition,
  setAddPagePosition,
  addPageCount,
  setAddPageCount,
}: PDFEditorContentProps) => {
  useEffect(() => {
    const path = files[0];
    if (!path) {
      setDocument(null);
      setState(createInitialState());
      setRectangle(null);
      return;
    }
    let cancelled = false;
    void invoke<PdfDocument>("inspect_pdf", { request: { path } })
      .then((metadata) => {
        if (cancelled) return;
        setDocument(metadata);
        setState(createInitialState(metadata));
        setRectangle(null);
      })
      .catch(() => {
        if (cancelled) return;
        setDocument(null);
        setState(createInitialState());
        setRectangle(null);
      });
    return () => {
      cancelled = true;
    };
  }, [files, setDocument, setRectangle, setState]);

  const editorAction = [
    "pdf-edit",
    "pdf-crop",
    "pdf-sign",
    "pdf-remove-pages",
    "pdf-organize",
    "pdf-page-numbers",
    "pdf-watermark",
    "pdf-extract-pages",
  ].includes(utility.id);
  const needsSelection = utility.id === "pdf-crop" || utility.id === "pdf-edit" || utility.id === "pdf-sign";
  const runDisabled =
    loading ||
    files.length === 0 ||
    (utility.id === "pdf-crop" && !rectangle) ||
    (utility.id === "pdf-remove-pages" && state.deletedPages.length === 0) ||
    (utility.id === "pdf-edit" && editMode !== "shape" && !text.trim()) ||
    (utility.id === "pdf-organize" && organizeMode === "add-pages" && (!Number.isInteger(addPageCount) || addPageCount < 1));

  return (
    <div className="pdf-editor-workspace">
      {document && editorAction ? (
        <PdfEditor
          document={document}
          state={state}
          onStateChange={setState}
          organizeControls={utility.id === "pdf-remove-pages" || (utility.id === "pdf-organize" && organizeMode === "arrange")}
          pageSelectionMode={utility.id === "pdf-remove-pages" ? "delete" : undefined}
          selectionRectangle={needsSelection ? rectangle : undefined}
          onSelectionChange={needsSelection ? setRectangle : undefined}
        />
      ) : (
        <div className="pdf-editor-empty">
          <strong>{document ? "PDF ready" : "Open a PDF to see its pages"}</strong>
          <span>
            {document
              ? "This outcome works from the selected file list below."
              : "The editor will show thumbnails, a page canvas, and an inspector here."}
          </span>
        </div>
      )}
      <div className="pdf-editor-controls">
        {utility.id === "pdf-edit" && (
          <>
            <div className="workspace-segmented-control" role="group" aria-label="Edit mode">
              {editModes.map((mode) => (
                <button key={mode} type="button" aria-pressed={editMode === mode} onClick={() => setEditMode(mode)}>
                  {mode[0].toUpperCase() + mode.slice(1)}
                </button>
              ))}
            </div>
            {editMode !== "shape" && <label className="workspace-field"><span>{editMode === "highlight" ? "Highlighted text" : "Text or note"}</span><input aria-label="Edit text" value={text} onChange={(event) => setText(event.target.value)} placeholder={editMode === "note" ? "Add a note" : "Type text to place"} /></label>}
            <label className="workspace-field"><span>Apply to pages</span><input aria-label="Edit pages" value={pages} onChange={(event) => setPages(event.target.value)} placeholder="All pages, or 1, 3" /></label>
          </>
        )}
        {utility.id === "pdf-crop" && <p className="workspace-note">Drag over the active page to choose the crop rectangle. Cropping hides content outside the selected area.</p>}
        {utility.id === "pdf-watermark" && <div className="workspace-field-grid"><label className="workspace-field"><span>Watermark text</span><input aria-label="Overlay text" value={watermarkText} onChange={(event) => setWatermarkText(event.target.value)} /></label><label className="workspace-field"><span>Opacity <output>{watermarkOpacity}%</output></span><input aria-label="Watermark opacity" type="range" min="1" max="100" value={watermarkOpacity} onChange={(event) => setWatermarkOpacity(Number(event.target.value))} /></label><label className="workspace-field"><span>Position</span><select aria-label="Watermark position" value={watermarkPosition} onChange={(event) => setWatermarkPosition(event.target.value)}><option value="center">Center</option><option value="top-left">Top left</option><option value="top-right">Top right</option><option value="bottom-left">Bottom left</option><option value="bottom-right">Bottom right</option></select></label><label className="workspace-field"><span>Pages</span><input aria-label="Watermark pages" value={pages} onChange={(event) => setPages(event.target.value)} placeholder="All pages, or 1, 3" /></label><button type="button" className="workspace-secondary-action" onClick={async () => { const picked = await open({ multiple: false, filters: [{ name: "Watermark image", extensions: ["png", "jpg", "jpeg"] }] }); if (typeof picked === "string") setWatermarkLogoPath(picked); }}>{watermarkLogoPath ? "Change logo" : "Add logo"}</button></div>}
        {utility.id === "pdf-sign" && <div className="workspace-field-grid"><label className="workspace-field"><span>Typed signature</span><input aria-label="Typed signature" value={text} onChange={(event) => setText(event.target.value)} /></label><button type="button" className="workspace-secondary-action" onClick={async () => { const picked = await open({ multiple: false, filters: [{ name: "Signature image", extensions: ["png", "jpg", "jpeg"] }] }); if (typeof picked === "string") setSignaturePath(picked); }}>{signaturePath ? "Change signature image" : "Use signature image"}</button></div>}
        {utility.id === "pdf-page-numbers" && <div className="workspace-field-grid"><label className="workspace-field"><span>Start at</span><input aria-label="Starting page number" type="number" min="1" value={startNumber} onChange={(event) => setStartNumber(Number(event.target.value))} /></label><label className="workspace-field"><span>Font size</span><input aria-label="Page number font size" type="number" min="8" max="72" value={fontSize} onChange={(event) => setFontSize(Number(event.target.value))} /></label><label className="workspace-field"><span>Position</span><select aria-label="Page number position" value={watermarkPosition} onChange={(event) => setWatermarkPosition(event.target.value)}><option value="bottom-right">Bottom right</option><option value="bottom-left">Bottom left</option><option value="top-right">Top right</option><option value="top-left">Top left</option></select></label><label className="workspace-field"><span>Pages</span><input aria-label="Page number pages" value={pages} onChange={(event) => setPages(event.target.value)} placeholder="All pages, or 1, 3" /></label></div>}
        {utility.id === "pdf-sign" && <p className="workspace-note">Drag on the active page to place the signature. Select an image or use the typed signature.</p>}
        {utility.id === "pdf-remove-pages" && <p className="workspace-note">Use Delete page in the editor. The original remains unchanged and the remaining pages keep their order.</p>}
        {utility.id === "pdf-organize" && (
          <>
            <div className="workspace-segmented-control" role="group" aria-label="PDF page operation">
              <button type="button" aria-pressed={organizeMode === "arrange"} onClick={() => setOrganizeMode("arrange")}>Arrange pages</button>
              <button type="button" aria-pressed={organizeMode === "add-pages"} onClick={() => setOrganizeMode("add-pages")}>Add blank pages</button>
            </div>
            {organizeMode === "add-pages" ? (
              <>
                <div className="workspace-field-grid">
                  <label className="workspace-field"><span>Insert</span><select aria-label="Blank page position" value={addPagePosition} onChange={(event) => setAddPagePosition(event.target.value as AddPagePosition)}><option value="before">Before selected page</option><option value="after">After selected page</option><option value="end">At end of document</option></select></label>
                  <label className="workspace-field"><span>Blank pages</span><input aria-label="Number of blank pages" type="number" min="1" max="100" value={addPageCount} onChange={(event) => setAddPageCount(Number(event.target.value))} /></label>
                </div>
                <p className="workspace-note">Blank pages copy the selected page size. The PDF is saved as a new copy and the original stays unchanged.</p>
              </>
            ) : <p className="workspace-note">Move, rotate, or delete pages in the editor, then save the resulting order.</p>}
          </>
        )}
        {utility.id === "pdf-merge" && <p className="workspace-note">Select two or more PDFs. They will be combined in the order shown in the file list.</p>}
        {utility.id === "pdf-split" && <div className="workspace-field-grid"><label className="workspace-field"><span>Split mode</span><select aria-label="Split mode" value={splitMode} onChange={(event) => setSplitMode(event.target.value as typeof splitMode)}><option value="pages">Every page</option><option value="ranges">Ranges</option><option value="chunks">Fixed-size chunks</option></select></label>{splitMode === "ranges" && <label className="workspace-field"><span>Ranges</span><input aria-label="Page ranges" value={pages} onChange={(event) => setPages(event.target.value)} placeholder="1-3, 4-8, 9-" /></label>}{splitMode === "chunks" && <label className="workspace-field"><span>Pages per file</span><input aria-label="Pages per file" type="number" min="1" value={chunkSize} onChange={(event) => setChunkSize(Number(event.target.value))} /></label>}</div>}
        {utility.id === "pdf-extract-pages" && <label className="workspace-field"><span>Pages or ranges</span><input aria-label="Page numbers or ranges" value={pages} onChange={(event) => setPages(event.target.value)} placeholder="1-3, 7" /></label>}
        {utility.id === "pdf-compress" && <label className="workspace-field"><span>Compression quality <output>{quality}%</output></span><input aria-label="PDF quality" type="range" min="1" max="100" value={quality} onChange={(event) => setQuality(Number(event.target.value))} /></label>}
        <button type="button" disabled={runDisabled} onClick={run} className="workspace-primary-action">{utility.id === "pdf-edit" ? "Edit PDF" : utility.id === "pdf-organize" && organizeMode === "add-pages" ? "Add Pages" : utility.shortTitle}</button>
      </div>
    </div>
  );
};
