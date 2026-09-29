import type { ToolDefinition } from "../contracts";
import { ToolScaffold } from "../components/ToolScaffold";

export const PlannedToolView = ({ utility }: { utility: ToolDefinition }) => (
    <ToolScaffold utility={utility} onRun={async () => []}>
        {() => (
            <div className="ds-callout ds-callout--warning">
                This tool is listed in the migration matrix. Its Tauri command and verification are not implemented yet.
            </div>
        )}
    </ToolScaffold>
);

export const UnavailableToolView = ({ utility }: { utility: ToolDefinition }) => {
    const mode = utility.id === "pdf-ocr" ? "PDF OCR" : utility.id === "image-blur-faces" ? "Face blur" : utility.id === "image-remove-bg" ? "Background removal" : utility.title;
    const explanation = "This tool requires an offline vision resource that is not bundled. Files will not be selected or processed.";
    return (
    <div className="flex h-full flex-col">
        <div className="mb-6">
            <h2 className="ds-view-title text-3xl font-bold">{utility.title}</h2>
            <p className="ds-view-description">{utility.blurb}</p>
        </div>
        <div className="ds-callout ds-callout--neutral" role="status">
            <p className="ds-callout__title">Unavailable in this build.</p>
            <p className="ds-callout__message">Mode: {mode}. {explanation}</p>
        </div>
    </div>
    );
};
