import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";
import { PdfEditor } from "../features/pdf-editor";
import type { CropPdfRequest, OrganizePdfRequest, PdfDocument, PdfEditorState, SignPdfRequest } from "../features/pdf-editor";

type Mode = "crop" | "sign" | "organize";

const initialState: PdfEditorState = { currentPage: 0, selectedPages: [], scope: { kind: "all" }, layout: "vertical" };

export const PDFPageToolView = ({ utility, mode }: { utility: ToolDefinition; mode: Mode }) => {
    const [document, setDocument] = useState<PdfDocument | null>(null);
    const [state, setState] = useState(initialState);
    const [text, setText] = useState("Signature");
    const [rectangle, setRectangle] = useState({ x: 0, y: 0, width: 612, height: 792 });

    return (
        <ToolScaffold
            utility={utility}
            onRun={(paths): Promise<ToolResult> => {
                const scope = state.scope.kind === "all" ? "all" : { selected: { pages: state.scope.pages } };
                if (mode === "crop") {
                    return invoke<ToolResult>("crop_pdf", { request: { paths, rectangle, scope, outputLocation: "alongsideInput" } satisfies CropPdfRequest });
                }
                if (mode === "sign") {
                    return invoke<ToolResult>("sign_pdf", { request: { paths, page: state.currentPage, text, rectangle, scope, outputLocation: "alongsideInput" } satisfies SignPdfRequest });
                }
                return invoke<ToolResult>("organize_pdf", { request: { paths, pageOrder: document?.pages.map((page) => page.index) ?? [], scope, outputLocation: "alongsideInput" } satisfies OrganizePdfRequest });
            }}
        >
            {({ files, run, loading }) => {
                useEffect(() => {
                    const path = files[0];
                    if (!path) { setDocument(null); return; }
                    invoke<PdfDocument>("inspect_pdf", { request: { path } }).then((metadata) => {
                        setDocument(metadata);
                        setState(initialState);
                        const page = metadata.pages[0];
                        if (page) setRectangle({ x: 0, y: 0, width: page.width, height: page.height });
                    }).catch(() => setDocument(null));
                }, [files]);

                return (
                    <div className="space-y-4">
                        {document ? <PdfEditor document={document} state={state} onStateChange={setState} /> : <p className="text-sm text-slate-500">Select a PDF to open the editor.</p>}
                        {mode === "sign" && <input aria-label="Typed signature" value={text} onChange={(event) => setText(event.target.value)} className="rounded border px-3 py-2" />}
                        <button type="button" disabled={loading || files.length === 0 || !document} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button>
                    </div>
                );
            }}
        </ToolScaffold>
    );
};

export const PDFCropView = (props: { utility: ToolDefinition }) => <PDFPageToolView {...props} mode="crop" />;
export const PDFSignView = (props: { utility: ToolDefinition }) => <PDFPageToolView {...props} mode="sign" />;
export const PDFOrganizeView = (props: { utility: ToolDefinition }) => <PDFPageToolView {...props} mode="organize" />;
