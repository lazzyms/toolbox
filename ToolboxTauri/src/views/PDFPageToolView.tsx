import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";
import { PdfEditor } from "../features/pdf-editor";
import type { CropPdfRequest, OrganizePdfRequest, PdfDocument, PdfEditorState, PdfRect, SignPdfRequest } from "../features/pdf-editor";

type Mode = "crop" | "sign" | "organize";

const initialState: PdfEditorState = { currentPage: 0, selectedPages: [], pageOrder: [], deletedPages: [], rotatePages: [], scope: { kind: "all" }, layout: "vertical", density: "comfortable", error: null };

export const PDFPageToolView = ({ utility, mode }: { utility: ToolDefinition; mode: Mode }) => {
    const [document, setDocument] = useState<PdfDocument | null>(null);
    const [state, setState] = useState(initialState);
    const [text, setText] = useState("Signature");
    const [signaturePath, setSignaturePath] = useState<string | null>(null);
    const [rectangle, setRectangle] = useState<PdfRect | null>(null);

    return (
        <ToolScaffold
            utility={utility}
            onRun={(paths): Promise<ToolResult> => {
                const scope = state.scope.kind === "all" ? "all" : { selected: { pages: state.scope.pages } };
                if (mode === "crop" || mode === "sign") {
                    if (!rectangle) {
                        return Promise.resolve([{ inputPath: paths[0] ?? "", outputPaths: [], detail: "", failure: { kind: "invalidInput", message: "Draw a crop rectangle first." } }]);
                    }
                    if (mode === "crop") {
                        return invoke<ToolResult>("crop_pdf", { request: { paths, rectangle, scope, outputLocation: "alongsideInput" } satisfies CropPdfRequest });
                    }
                    return invoke<ToolResult>("sign_pdf", { request: { paths, page: state.currentPage, text, signaturePath, rectangle, scope, outputLocation: "alongsideInput" } satisfies SignPdfRequest });
                }
                return invoke<ToolResult>("organize_pdf", { request: { paths, pageOrder: state.pageOrder, deletePages: state.deletedPages, rotatePages: state.rotatePages, scope, outputLocation: "alongsideInput" } satisfies OrganizePdfRequest });
            }}
        >
            {({ files, run, loading }) => <PDFPageToolContent files={files} run={run} loading={loading} utility={utility} mode={mode} document={document} setDocument={setDocument} state={state} setState={setState} text={text} setText={setText} signaturePath={signaturePath} setSignaturePath={setSignaturePath} rectangle={rectangle} setRectangle={setRectangle} />}
        </ToolScaffold>
    );
};

const PDFPageToolContent = ({ files, run, loading, utility, mode, document, setDocument, state, setState, text, setText, signaturePath, setSignaturePath, rectangle, setRectangle }: {
    files: string[]; run: () => Promise<void>; loading: boolean; utility: ToolDefinition; mode: Mode;
    document: PdfDocument | null; setDocument: (document: PdfDocument | null) => void; state: PdfEditorState;
    setState: (state: PdfEditorState) => void; text: string; setText: (text: string) => void; signaturePath: string | null;
    setSignaturePath: (path: string | null) => void;
    rectangle: PdfRect | null; setRectangle: (rectangle: PdfRect | null) => void;
}) => {
    useEffect(() => {
        const path = files[0];
        if (!path) { setDocument(null); setRectangle(null); return; }
        invoke<PdfDocument>("inspect_pdf", { request: { path } }).then((metadata) => {
            setDocument(metadata);
            setState({ ...initialState, pageOrder: metadata.pages.map((page) => page.index) });
            const page = metadata.pages[0];
            if (page) setRectangle(mode === "crop" ? null : { x: page.x ?? 0, y: page.y ?? 0, width: page.width, height: page.height });
        }).catch(() => { setDocument(null); setRectangle(null); });
    }, [files, mode, setDocument, setState, setRectangle]);

    return <div className="space-y-4">
        {document ? <PdfEditor document={document} state={state} onStateChange={setState} organizeControls={mode === "organize"} selectionRectangle={mode === "crop" ? rectangle : undefined} onSelectionChange={mode === "crop" ? setRectangle : undefined} /> : <p className="text-sm text-slate-500">Select a PDF to open the editor.</p>}
        {mode === "sign" && <div className="flex flex-wrap items-center gap-2"><input aria-label="Typed signature" value={text} onChange={(event) => setText(event.target.value)} className="rounded border px-3 py-2" /><button type="button" onClick={async () => { const picked = await open({ multiple: false, filters: [{ name: "Signature image", extensions: ["png", "jpg", "jpeg"] }] }); if (typeof picked === "string") setSignaturePath(picked); }} className="rounded border px-3 py-2">{signaturePath ? "Change signature image" : "Use signature image"}</button>{signaturePath && <span className="text-sm text-slate-600" aria-live="polite">Image signature selected</span>}</div>}
        <button type="button" disabled={loading || files.length === 0 || !document || (mode === "crop" && !rectangle)} onClick={run} className="rounded-lg bg-blue-600 px-6 py-2 font-medium text-white disabled:opacity-50">{utility.shortTitle}</button>
    </div>;
};

export const PDFCropView = (props: { utility: ToolDefinition }) => <PDFPageToolView {...props} mode="crop" />;
export const PDFSignView = (props: { utility: ToolDefinition }) => <PDFPageToolView {...props} mode="sign" />;
export const PDFOrganizeView = (props: { utility: ToolDefinition }) => <PDFPageToolView {...props} mode="organize" />;
