import type { ToolDefinition } from "../contracts";
import { ToolScaffold } from "../components/ToolScaffold";
import { Badge } from "../components/ui/badge";
import { Card } from "../components/ui/card";

export const PlannedToolView = ({ utility }: { utility: ToolDefinition }) => (
    <ToolScaffold utility={utility} onRun={async () => []}>
        {() => (
            <Card className="planned-tool-callout">
                <Badge variant="outline">Planned</Badge>
                This tool is listed in the migration matrix. Its Tauri command and verification are not implemented yet.
            </Card>
        )}
    </ToolScaffold>
);

export const UnavailableToolView = ({ utility }: { utility: ToolDefinition }) => {
    const mode = utility.id === "pdf-ocr" ? "PDF OCR" : utility.id === "image-blur-faces" ? "Face blur" : utility.id === "image-remove-bg" ? "Background removal" : utility.title;
    const explanation = "This tool requires an offline vision resource that is not bundled. Files will not be selected or processed.";
    return (
    <div className="flex h-full flex-col">
        <div className="mb-6">
            <h2 className="text-3xl font-bold">{utility.title}</h2>
            <p className="text-muted-foreground">{utility.blurb}</p>
        </div>
        <Card className="planned-tool-callout" role="status">
            <Badge variant="outline">Unavailable in this build.</Badge>
            <p>Mode: {mode}. {explanation}</p>
        </Card>
    </div>
    );
};
