import type { ToolDefinition } from "../contracts";
import type { IntakeCopy } from "../components/intake";
import { extensionOf } from "../components/intake";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

export type PlainTextOptions = {
  pageSize: "a4" | "letter" | "legal";
  margins: "narrow" | "normal" | "wide";
  font: "monospace" | "sans" | "serif";
  encoding: "detect" | "utf8" | "utf16le" | "utf16be" | "windows1252";
  wrapLines: boolean;
};

export const defaultPlainTextOptions: PlainTextOptions = { pageSize: "a4", margins: "normal", font: "monospace", encoding: "detect", wrapLines: true };

export type MarkdownOptions = {
  pageSize: "a4" | "letter" | "legal";
  margins: "narrow" | "normal" | "wide";
  bodyFont: "sans" | "serif" | "monospace";
  codeStyle: "shaded" | "plain";
  tableOfContents: boolean;
};

export const defaultMarkdownOptions: MarkdownOptions = { pageSize: "a4", margins: "normal", bodyFont: "sans", codeStyle: "shaded", tableOfContents: false };

export type TableOptions = {
  delimiter: "auto" | "comma" | "semicolon" | "tab" | "pipe";
  headerRow: boolean;
  encoding: PlainTextOptions["encoding"];
  orientation: "landscape" | "portrait";
  fit: "width" | "none";
};

export const defaultTableOptions: TableOptions = { delimiter: "auto", headerRow: true, encoding: "detect", orientation: "landscape", fit: "width" };

export type SpreadsheetOptions = { sheets: "all" | "visible"; printAreas: boolean; fitToWidth: boolean };

export const defaultSpreadsheetOptions: SpreadsheetOptions = { sheets: "all", printAreas: true, fitToWidth: true };

export type PresentationOptions = { speakerNotes: boolean; hiddenSlides: boolean };

export const defaultPresentationOptions: PresentationOptions = { speakerNotes: false, hiddenSlides: false };

export type WebPageOptions = { pageSize: "a4" | "letter" | "legal"; margins: "narrow" | "normal" | "wide"; backgroundGraphics: boolean };

export const defaultWebPageOptions: WebPageOptions = { pageSize: "a4", margins: "normal", backgroundGraphics: true };

export type EbookOptions = { pageSize: "a4" | "letter" | "legal"; fontScale: number };

export const defaultEbookOptions: EbookOptions = { pageSize: "a4", fontScale: 100 };

export const isPlainText = (path: string) => extensionOf(path) === ".txt";

export const isMarkdown = (path: string) => [".md", ".markdown"].includes(extensionOf(path));

export const isTable = (path: string) => [".csv", ".tsv"].includes(extensionOf(path));

/** Excel 97-2003 workbooks take the same options, read in basic fidelity. */
export const isSpreadsheet = (path: string) => [".xlsx", ".ods", ".xls"].includes(extensionOf(path));

/** PowerPoint 97-2003 presentations take the same options, read in basic fidelity. */
export const isPresentation = (path: string) => [".pptx", ".odp", ".ppt"].includes(extensionOf(path));

/** HTML files and pasted markup, which stands in the list as an .html item. */
export const isWebPage = (path: string) => [".html", ".htm"].includes(extensionOf(path));

/** MOBI books take the same options, read in basic fidelity. */
export const isEbook = (path: string) => [".epub", ".mobi"].includes(extensionOf(path));

export type ConvertToPdfOptions = { plainText: PlainTextOptions; markdown: MarkdownOptions; tables: TableOptions; spreadsheets: SpreadsheetOptions; presentations: PresentationOptions; webPages: WebPageOptions; ebooks: EbookOptions };

export const defaultConvertToPdfOptions: ConvertToPdfOptions = { plainText: defaultPlainTextOptions, markdown: defaultMarkdownOptions, tables: defaultTableOptions, spreadsheets: defaultSpreadsheetOptions, presentations: defaultPresentationOptions, webPages: defaultWebPageOptions, ebooks: defaultEbookOptions };

export const convertToPdfRequest = (paths: string[], outputLocation: unknown, options: ConvertToPdfOptions) => ({
  paths,
  outputLocation,
  ...(paths.some(isPlainText) ? { plainText: options.plainText } : {}),
  ...(paths.some(isMarkdown) ? { markdown: options.markdown } : {}),
  ...(paths.some(isTable) ? { tables: options.tables } : {}),
  ...(paths.some(isSpreadsheet) ? { spreadsheets: options.spreadsheets } : {}),
  ...(paths.some(isPresentation) ? { presentations: options.presentations } : {}),
  ...(paths.some(isWebPage) ? { webPages: options.webPages } : {}),
  ...(paths.some(isEbook) ? { ebooks: options.ebooks } : {}),
});

const unavailableFamily = (tool: ToolDefinition, path: string) =>
  tool.capability.formatFamilies?.find((family) => family.nativeAvailability === "unavailable" && family.extensions.includes(extensionOf(path)));

const basicFidelity = (tool: ToolDefinition, path: string) =>
  tool.capability.formatFamilies?.some((family) => family.basicFidelity && family.nativeAvailability === "available" && family.extensions.includes(extensionOf(path))) ?? false;

const fileCount = (count: number) => `${count} ${count === 1 ? "file" : "files"}`;

export const convertToPdfIntakeCopy = (tool: ToolDefinition): IntakeCopy => ({
  notAccepted: "This file type can't become a PDF here.",
  unsupported: () => "This file type can't become a PDF here.",
  willFail: (path) => {
    if (extensionOf(path) === ".pdf") return "Already a PDF";
    return unavailableFamily(tool, path) ? `${extensionOf(path).slice(1).toUpperCase()} comes in a later update` : null;
  },
  flagLabel: (path) => (extensionOf(path) === ".pdf" ? "Will be skipped" : "Not available yet"),
  mark: (path) => (basicFidelity(tool, path) ? "Basic fidelity" : null),
  flaggedNotice: (count) => `${fileCount(count)} won't become ${count === 1 ? "a PDF" : "PDFs"}. Each one says why on its row.`,
  countHint: (flagged, total) => {
    const runnable = total - flagged;
    return flagged > 0 ? `This will make ${runnable} ${runnable === 1 ? "PDF" : "PDFs"}, ${flagged} skipped` : `This will make ${total} ${total === 1 ? "PDF" : "PDFs"}`;
  },
});

const appliesTo = (files: string[], plural: string) => `Applies to ${files.length === 1 ? files[0].split(/[\\/]/).pop() : `${files.length} ${plural}`}`;

type PanelProps<T> = { files: string[]; options: T; onChange: (options: T) => void; disabled: boolean };

export const ConvertToPdfOptionsPanel = ({ files, options, onChange, disabled }: PanelProps<ConvertToPdfOptions>) => {
  if (![isSpreadsheet, isPresentation, isPlainText, isMarkdown, isWebPage, isEbook, isTable].some((matches) => files.some(matches))) {
    return <p className="workspace-note">Options for each format appear here when a file of that format is in the list.</p>;
  }
  return (
    <>
      <SpreadsheetOptionsPanel files={files} options={options.spreadsheets} onChange={(spreadsheets) => onChange({ ...options, spreadsheets })} disabled={disabled} />
      <PresentationOptionsPanel files={files} options={options.presentations} onChange={(presentations) => onChange({ ...options, presentations })} disabled={disabled} />
      <MarkdownOptionsPanel files={files} options={options.markdown} onChange={(markdown) => onChange({ ...options, markdown })} disabled={disabled} />
      <WebPageOptionsPanel files={files} options={options.webPages} onChange={(webPages) => onChange({ ...options, webPages })} disabled={disabled} />
      <EbookOptionsPanel files={files} options={options.ebooks} onChange={(ebooks) => onChange({ ...options, ebooks })} disabled={disabled} />
      <PlainTextOptionsPanel files={files} options={options.plainText} onChange={(plainText) => onChange({ ...options, plainText })} disabled={disabled} />
      <TablesOptionsPanel files={files} options={options.tables} onChange={(tables) => onChange({ ...options, tables })} disabled={disabled} />
    </>
  );
};

const SpreadsheetOptionsPanel = ({ files, options, onChange, disabled }: PanelProps<SpreadsheetOptions>) => {
  const spreadsheetFiles = files.filter(isSpreadsheet);
  if (spreadsheetFiles.length === 0) return null;
  const set = <K extends keyof SpreadsheetOptions>(key: K, value: SpreadsheetOptions[K]) => onChange({ ...options, [key]: value });
  return (
    <fieldset role="region" aria-label="Spreadsheets options" className="grid gap-3" disabled={disabled}>
      <div className="workspace-panel-intro">
        <p className="workspace-panel-label">Spreadsheets</p>
        <p className="workspace-panel-copy">{appliesTo(spreadsheetFiles, "spreadsheets")}</p>
      </div>
      <div className="workspace-field">
        <Label id="spreadsheets-sheets-label">Sheets</Label>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          aria-labelledby="spreadsheets-sheets-label"
          value={options.sheets}
          onValueChange={(value) => {
            if (value) set("sheets", value as SpreadsheetOptions["sheets"]);
          }}
        >
          <ToggleGroupItem value="all">All sheets</ToggleGroupItem>
          <ToggleGroupItem value="visible">Visible only</ToggleGroupItem>
        </ToggleGroup>
      </div>
      <Label htmlFor="spreadsheets-print-areas" className="cursor-pointer">
        <Checkbox id="spreadsheets-print-areas" checked={options.printAreas} onCheckedChange={(checked) => set("printAreas", checked === true)} />
        Respect print areas
      </Label>
      <Label htmlFor="spreadsheets-fit-to-width" className="cursor-pointer">
        <Checkbox id="spreadsheets-fit-to-width" checked={options.fitToWidth} onCheckedChange={(checked) => set("fitToWidth", checked === true)} />
        Fit each sheet to the page width
      </Label>
    </fieldset>
  );
};

const PresentationOptionsPanel = ({ files, options, onChange, disabled }: PanelProps<PresentationOptions>) => {
  const presentationFiles = files.filter(isPresentation);
  if (presentationFiles.length === 0) return null;
  const set = <K extends keyof PresentationOptions>(key: K, value: PresentationOptions[K]) => onChange({ ...options, [key]: value });
  return (
    <fieldset role="region" aria-label="Presentations options" className="grid gap-3" disabled={disabled}>
      <div className="workspace-panel-intro">
        <p className="workspace-panel-label">Presentations</p>
        <p className="workspace-panel-copy">{appliesTo(presentationFiles, "presentations")}</p>
      </div>
      <Label htmlFor="presentations-speaker-notes" className="cursor-pointer">
        <Checkbox id="presentations-speaker-notes" checked={options.speakerNotes} onCheckedChange={(checked) => set("speakerNotes", checked === true)} />
        Include speaker notes
      </Label>
      <Label htmlFor="presentations-hidden-slides" className="cursor-pointer">
        <Checkbox id="presentations-hidden-slides" checked={options.hiddenSlides} onCheckedChange={(checked) => set("hiddenSlides", checked === true)} />
        Include hidden slides
      </Label>
    </fieldset>
  );
};

const MarkdownOptionsPanel = ({ files, options, onChange, disabled }: PanelProps<MarkdownOptions>) => {
  const markdownFiles = files.filter(isMarkdown);
  if (markdownFiles.length === 0) return null;
  const set = <K extends keyof MarkdownOptions>(key: K, value: MarkdownOptions[K]) => onChange({ ...options, [key]: value });
  return (
    <fieldset role="region" aria-label="Markdown options" className="grid gap-3" disabled={disabled}>
      <div className="workspace-panel-intro">
        <p className="workspace-panel-label">Markdown</p>
        <p className="workspace-panel-copy">{appliesTo(markdownFiles, "Markdown files")}</p>
      </div>
      <div className="workspace-field-grid">
        <div className="workspace-field">
          <Label htmlFor="markdown-page-size">Page size</Label>
          <NativeSelect id="markdown-page-size" value={options.pageSize} onChange={(event) => set("pageSize", event.target.value as MarkdownOptions["pageSize"])}>
            <option value="a4">A4 (default)</option>
            <option value="letter">Letter</option>
            <option value="legal">Legal</option>
          </NativeSelect>
        </div>
        <div className="workspace-field">
          <Label htmlFor="markdown-margins">Margins</Label>
          <NativeSelect id="markdown-margins" value={options.margins} onChange={(event) => set("margins", event.target.value as MarkdownOptions["margins"])}>
            <option value="normal">Default</option>
            <option value="narrow">Narrow</option>
            <option value="wide">Wide</option>
          </NativeSelect>
        </div>
        <div className="workspace-field">
          <Label htmlFor="markdown-body-font">Body font</Label>
          <NativeSelect id="markdown-body-font" value={options.bodyFont} onChange={(event) => set("bodyFont", event.target.value as MarkdownOptions["bodyFont"])}>
            <option value="sans">System sans-serif</option>
            <option value="serif">System serif</option>
            <option value="monospace">Monospace</option>
          </NativeSelect>
        </div>
        <div className="workspace-field">
          <Label id="markdown-code-style-label">Code blocks</Label>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            aria-labelledby="markdown-code-style-label"
            value={options.codeStyle}
            onValueChange={(value) => {
              if (value) set("codeStyle", value as MarkdownOptions["codeStyle"]);
            }}
          >
            <ToggleGroupItem value="shaded">Shaded</ToggleGroupItem>
            <ToggleGroupItem value="plain">Plain</ToggleGroupItem>
          </ToggleGroup>
        </div>
      </div>
      <div className="grid gap-1">
        <Label htmlFor="markdown-table-of-contents" className="cursor-pointer">
          <Checkbox id="markdown-table-of-contents" aria-describedby="markdown-table-of-contents-hint" checked={options.tableOfContents} onCheckedChange={(checked) => set("tableOfContents", checked === true)} />
          Add a table of contents
        </Label>
        <p id="markdown-table-of-contents-hint" className="workspace-note">Built from the headings, with links to each section.</p>
      </div>
    </fieldset>
  );
};

const WebPageOptionsPanel = ({ files, options, onChange, disabled }: PanelProps<WebPageOptions>) => {
  const pages = files.filter(isWebPage);
  if (pages.length === 0) return null;
  const set = <K extends keyof WebPageOptions>(key: K, value: WebPageOptions[K]) => onChange({ ...options, [key]: value });
  return (
    <fieldset role="region" aria-label="Web pages options" className="grid gap-3" disabled={disabled}>
      <div className="workspace-panel-intro">
        <p className="workspace-panel-label">Web pages</p>
        <p className="workspace-panel-copy">{appliesTo(pages, "web pages")}</p>
      </div>
      <div className="workspace-field-grid">
        <div className="workspace-field">
          <Label htmlFor="web-pages-page-size">Page size</Label>
          <NativeSelect id="web-pages-page-size" value={options.pageSize} onChange={(event) => set("pageSize", event.target.value as WebPageOptions["pageSize"])}>
            <option value="a4">A4 (default)</option>
            <option value="letter">Letter</option>
            <option value="legal">Legal</option>
          </NativeSelect>
        </div>
        <div className="workspace-field">
          <Label htmlFor="web-pages-margins">Margins</Label>
          <NativeSelect id="web-pages-margins" value={options.margins} onChange={(event) => set("margins", event.target.value as WebPageOptions["margins"])}>
            <option value="normal">Default</option>
            <option value="narrow">Narrow</option>
            <option value="wide">Wide</option>
          </NativeSelect>
        </div>
      </div>
      <div className="grid gap-1">
        <Label htmlFor="web-pages-background" className="cursor-pointer">
          <Checkbox id="web-pages-background" aria-describedby="web-pages-background-hint" checked={options.backgroundGraphics} onCheckedChange={(checked) => set("backgroundGraphics", checked === true)} />
          Print background colors
        </Label>
        <p id="web-pages-background-hint" className="workspace-note">Scripts never run, and nothing is loaded from the internet. Stylesheets and images next to the file are used.</p>
      </div>
    </fieldset>
  );
};

const EbookOptionsPanel = ({ files, options, onChange, disabled }: PanelProps<EbookOptions>) => {
  const books = files.filter(isEbook);
  if (books.length === 0) return null;
  const set = <K extends keyof EbookOptions>(key: K, value: EbookOptions[K]) => onChange({ ...options, [key]: value });
  return (
    <fieldset role="region" aria-label="eBooks options" className="grid gap-3" disabled={disabled}>
      <div className="workspace-panel-intro">
        <p className="workspace-panel-label">eBooks</p>
        <p className="workspace-panel-copy">{appliesTo(books, "eBooks")}</p>
      </div>
      <div className="workspace-field-grid">
        <div className="workspace-field">
          <Label htmlFor="ebooks-page-size">Page size</Label>
          <NativeSelect id="ebooks-page-size" value={options.pageSize} onChange={(event) => set("pageSize", event.target.value as EbookOptions["pageSize"])}>
            <option value="a4">A4 (default)</option>
            <option value="letter">Letter</option>
            <option value="legal">Legal</option>
          </NativeSelect>
        </div>
        <div className="workspace-field">
          <Label htmlFor="ebooks-text-size">Text size</Label>
          <NativeSelect id="ebooks-text-size" aria-describedby="ebooks-hint" value={String(options.fontScale)} onChange={(event) => set("fontScale", Number(event.target.value))}>
            {[75, 90, 100, 110, 125, 150].map((scale) => <option key={scale} value={scale}>{scale === 100 ? "100% (default)" : `${scale}%`}</option>)}
          </NativeSelect>
        </div>
      </div>
      <p id="ebooks-hint" className="workspace-note">Each chapter starts a new page, and the book's table of contents becomes bookmarks. Copy-protected (DRM) eBooks can't be converted.</p>
    </fieldset>
  );
};

const PlainTextOptionsPanel = ({ files, options, onChange, disabled }: PanelProps<PlainTextOptions>) => {
  const textFiles = files.filter(isPlainText);
  if (textFiles.length === 0) return null;
  const set = <K extends keyof PlainTextOptions>(key: K, value: PlainTextOptions[K]) => onChange({ ...options, [key]: value });
  return (
    <fieldset role="region" aria-label="Plain text options" className="grid gap-3" disabled={disabled}>
      <div className="workspace-panel-intro">
        <p className="workspace-panel-label">Plain text</p>
        <p className="workspace-panel-copy">{appliesTo(textFiles, "text files")}</p>
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

const TablesOptionsPanel = ({ files, options, onChange, disabled }: PanelProps<TableOptions>) => {
  const tableFiles = files.filter(isTable);
  if (tableFiles.length === 0) return null;
  const set = <K extends keyof TableOptions>(key: K, value: TableOptions[K]) => onChange({ ...options, [key]: value });
  return (
    <fieldset role="region" aria-label="Tables options" className="grid gap-3" disabled={disabled}>
      <div className="workspace-panel-intro">
        <p className="workspace-panel-label">Tables</p>
        <p className="workspace-panel-copy">{appliesTo(tableFiles, "table files")}</p>
      </div>
      <div className="workspace-field-grid">
        <div className="workspace-field">
          <Label htmlFor="tables-delimiter">Delimiter</Label>
          <NativeSelect id="tables-delimiter" aria-describedby="tables-delimiter-hint" value={options.delimiter} onChange={(event) => set("delimiter", event.target.value as TableOptions["delimiter"])}>
            <option value="auto">By file type</option>
            <option value="comma">Comma</option>
            <option value="semicolon">Semicolon</option>
            <option value="tab">Tab</option>
            <option value="pipe">Pipe</option>
          </NativeSelect>
          <p id="tables-delimiter-hint" className="workspace-note">Comma for CSV and tab for TSV, unless you pick one.</p>
        </div>
        <div className="workspace-field">
          <Label htmlFor="tables-encoding">Encoding</Label>
          <NativeSelect id="tables-encoding" value={options.encoding} onChange={(event) => set("encoding", event.target.value as TableOptions["encoding"])}>
            <option value="detect">Detect</option>
            <option value="utf8">UTF-8</option>
            <option value="utf16le">UTF-16 LE</option>
            <option value="utf16be">UTF-16 BE</option>
            <option value="windows1252">Windows-1252</option>
          </NativeSelect>
        </div>
      </div>
      <Label htmlFor="tables-header-row" className="cursor-pointer">
        <Checkbox id="tables-header-row" checked={options.headerRow} onCheckedChange={(checked) => set("headerRow", checked === true)} />
        First row is a header, repeated on each page
      </Label>
      <div className="workspace-field-grid">
        <div className="workspace-field">
          <Label htmlFor="tables-orientation">Orientation</Label>
          <NativeSelect id="tables-orientation" value={options.orientation} onChange={(event) => set("orientation", event.target.value as TableOptions["orientation"])}>
            <option value="landscape">Landscape</option>
            <option value="portrait">Portrait</option>
          </NativeSelect>
        </div>
        <div className="workspace-field">
          <Label htmlFor="tables-fit">Fit</Label>
          <NativeSelect id="tables-fit" value={options.fit} onChange={(event) => set("fit", event.target.value as TableOptions["fit"])}>
            <option value="width">Fit to width</option>
            <option value="none">Keep text size</option>
          </NativeSelect>
        </div>
      </div>
    </fieldset>
  );
};
