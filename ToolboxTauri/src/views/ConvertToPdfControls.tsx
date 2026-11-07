import type { ToolDefinition } from "../contracts";
import type { IntakeCopy } from "../components/intake";
import { extensionOf } from "../components/intake";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";

export type PlainTextOptions = {
  pageSize: "a4" | "letter" | "legal";
  margins: "narrow" | "normal" | "wide";
  font: "monospace" | "sans" | "serif";
  encoding: "detect" | "utf8" | "utf16le" | "utf16be" | "windows1252";
  wrapLines: boolean;
};

export const defaultPlainTextOptions: PlainTextOptions = { pageSize: "a4", margins: "normal", font: "monospace", encoding: "detect", wrapLines: true };

export const isPlainText = (path: string) => extensionOf(path) === ".txt";

export const convertToPdfRequest = (paths: string[], outputLocation: unknown, plainText: PlainTextOptions) => ({
  paths,
  outputLocation,
  ...(paths.some(isPlainText) ? { plainText } : {}),
});

const unavailableFamily = (tool: ToolDefinition, path: string) =>
  tool.capability.formatFamilies?.find((family) => family.nativeAvailability === "unavailable" && family.extensions.includes(extensionOf(path)));

const fileCount = (count: number) => `${count} ${count === 1 ? "file" : "files"}`;

export const convertToPdfIntakeCopy = (tool: ToolDefinition): IntakeCopy => ({
  notAccepted: "This file type can't become a PDF here.",
  unsupported: () => "This file type can't become a PDF here.",
  willFail: (path) => {
    if (extensionOf(path) === ".pdf") return "Already a PDF";
    return unavailableFamily(tool, path) ? `${extensionOf(path).slice(1).toUpperCase()} comes in a later update` : null;
  },
  flagLabel: (path) => (extensionOf(path) === ".pdf" ? "Will be skipped" : "Not available yet"),
  flaggedNotice: (count) => `${fileCount(count)} won't become ${count === 1 ? "a PDF" : "PDFs"}. Each one says why on its row.`,
  countHint: (flagged, total) => {
    const runnable = total - flagged;
    return flagged > 0 ? `This will make ${runnable} ${runnable === 1 ? "PDF" : "PDFs"}, ${flagged} skipped` : `This will make ${total} ${total === 1 ? "PDF" : "PDFs"}`;
  },
});

export const PlainTextOptionsPanel = ({ files, options, onChange, disabled }: { files: string[]; options: PlainTextOptions; onChange: (options: PlainTextOptions) => void; disabled: boolean }) => {
  const textFiles = files.filter(isPlainText);
  if (textFiles.length === 0) {
    return <p className="workspace-note">Options for each format appear here when a file of that format is in the list.</p>;
  }
  const set = <K extends keyof PlainTextOptions>(key: K, value: PlainTextOptions[K]) => onChange({ ...options, [key]: value });
  return (
    <fieldset role="region" aria-label="Plain text options" className="grid gap-3" disabled={disabled}>
      <div className="workspace-panel-intro">
        <p className="workspace-panel-label">Plain text</p>
        <p className="workspace-panel-copy">Applies to {textFiles.length === 1 ? textFiles[0].split(/[\\/]/).pop() : `${textFiles.length} text files`}</p>
      </div>
      <div className="workspace-field-grid">
        <div className="workspace-field">
          <Label htmlFor="plain-text-page-size">Page size</Label>
          <NativeSelect id="plain-text-page-size" value={options.pageSize} onChange={(event) => set("pageSize", event.target.value as PlainTextOptions["pageSize"])}>
            <option value="a4">A4 (default)</option>
            <option value="letter">Letter</option>
            <option value="legal">Legal</option>
          </NativeSelect>
        </div>
        <div className="workspace-field">
          <Label htmlFor="plain-text-margins">Margins</Label>
          <NativeSelect id="plain-text-margins" value={options.margins} onChange={(event) => set("margins", event.target.value as PlainTextOptions["margins"])}>
            <option value="narrow">Narrow</option>
            <option value="normal">Normal</option>
            <option value="wide">Wide</option>
          </NativeSelect>
        </div>
        <div className="workspace-field">
          <Label htmlFor="plain-text-font">Font</Label>
          <NativeSelect id="plain-text-font" value={options.font} onChange={(event) => set("font", event.target.value as PlainTextOptions["font"])}>
            <option value="monospace">Monospace</option>
            <option value="sans">Sans-serif</option>
            <option value="serif">Serif</option>
          </NativeSelect>
        </div>
        <div className="workspace-field">
          <Label htmlFor="plain-text-encoding">Encoding</Label>
          <NativeSelect id="plain-text-encoding" aria-describedby="plain-text-encoding-hint" value={options.encoding} onChange={(event) => set("encoding", event.target.value as PlainTextOptions["encoding"])}>
            <option value="detect">Detect</option>
            <option value="utf8">UTF-8</option>
            <option value="utf16le">UTF-16 LE</option>
            <option value="utf16be">UTF-16 BE</option>
            <option value="windows1252">Windows-1252</option>
          </NativeSelect>
          <p id="plain-text-encoding-hint" className="workspace-note">Pick one if characters look wrong.</p>
        </div>
      </div>
      <Label htmlFor="plain-text-wrap" className="cursor-pointer">
        <Checkbox id="plain-text-wrap" checked={options.wrapLines} onCheckedChange={(checked) => set("wrapLines", checked === true)} />
        Wrap long lines
      </Label>
    </fieldset>
  );
};
