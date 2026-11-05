import { useId } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import type { JobOutcome } from "../../contracts";
import type { ExistingPdfUtilityId } from "../../views/PDFEditorWorkspaceView";
import { shortenHomePath, useHomeDir } from "../../homePath";
import { parentFolder } from "../../components/intake";
import { fileName, rowView } from "../../components/resultRow";
import type { PdfEditorBatchTool, ReportActions } from "./batchTools";

/** The report `inspect_pdf_privacy` returns in each succeeded outcome's detail. */
interface PdfProperties {
  title: string | null;
  author: string | null;
  subject: string | null;
  keywords: string | null;
  creator: string | null;
  producer: string | null;
  creationDate: string | null;
  modificationDate: string | null;
}

type ActionTrigger = "documentScript" | "open" | "beforeClose" | "beforeSave" | "afterSave" | "beforePrint" | "afterPrint" | "pageOpen" | "pageClose" | "annotation";

type Permission = "print" | "printHighQuality" | "modify" | "copy" | "copyForAccessibility" | "annotate" | "fillForms" | "assemble";

export interface PrivacyReportData {
  pageCount: number;
  documentInformation: PdfProperties;
  xmp: {
    summary: (PdfProperties & { properties: string[] }) | null;
    conflicts: { property: keyof PdfProperties; info: string; xmp: string }[];
  };
  encryption: {
    openPassword: boolean;
    version: number;
    revision: number;
    keyBits: number | null;
    encryptsMetadata: boolean;
    allowed: Permission[];
    denied: Permission[];
  } | null;
  identifiers: {
    documentId: { permanent: string; changing: string; unchanged: boolean } | null;
    producer: { name: string; version: string | null } | null;
  };
  incrementalUpdates: number | null;
  embeddedFiles: { name: string; size: number | null; mimeType: string | null; page: number | null }[];
  actions: { trigger: ActionTrigger; page: number | null; name: string | null; kinds: string[] }[];
  annotations: { subtype: string; count: number; authors: string[] }[];
  filledFormFields: string[];
  optionalContent: {
    layers: { name: string; visible: boolean }[];
    hidden: { page: number; subtype: string; reason: "hidden" | "offPage" }[];
  };
  links: { host: string; count: number }[];
  signatures: { field: string; signer: string | null; signedAt: string | null }[];
  leftovers: { thumbnailPages: number[]; pagePieces: { page: number; applications: string[] }[] };
  unsupported: string[];
}

type Severity = "high" | "medium" | "low";
type Item = { key: string; value: string };
type Pointer = { label: string; tool: ExistingPdfUtilityId };
/** `found` is null when nothing was found, and "unchecked" when the category couldn't be read. */
type Category = { name: string; severity: Severity; why: string; found: { count: string; items: Item[] } | null | "unchecked"; pointer?: Pointer };

const severityLabel: Record<Severity, string> = { high: "High", medium: "Medium", low: "Low" };
const severityVariant = { high: "destructive", medium: "secondary", low: "outline" } as const;

const propertyLabels: Record<keyof PdfProperties, string> = {
  title: "Title",
  author: "Author",
  subject: "Subject",
  keywords: "Keywords",
  creator: "Created with",
  producer: "Saved as PDF by",
  creationDate: "Created",
  modificationDate: "Modified",
};

const permissionLabels: Record<Permission, string> = {
  print: "Printing",
  printHighQuality: "High-quality printing",
  modify: "Editing",
  copy: "Copying text",
  copyForAccessibility: "Copying for accessibility",
  annotate: "Adding comments",
  fillForms: "Filling forms",
  assemble: "Assembling pages",
};

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;
/** Dates arrive as ISO 8601 text; the report shows the date and the minute. */
const shownDate = (value: string) => value.slice(0, 16).replace("T", " ");
const shownValue = (key: keyof PdfProperties, value: string) => key === "creationDate" || key === "modificationDate" ? shownDate(value) : value;
const sentence = (labels: string[]) => labels.map((label, index) => index === 0 ? label : label.toLowerCase()).join(", ");
const humanized = (name: string) => name.replace(/(?<=[a-z])(?=[A-Z])/g, " ").toLowerCase().replace(/^./, (first) => first.toUpperCase());
const pages = (numbers: number[]) => `${numbers.length === 1 ? "Page" : "Pages"} ${numbers.join(", ")}`;
const sum = (counts: { count: number }[]) => counts.reduce((total, { count }) => total + count, 0);
const fileSize = (bytes: number) => bytes < 1024 ? plural(bytes, "byte", "bytes") : bytes < 1024 * 1024 ? `${Math.round(bytes / 1024)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;

const triggerLabels: Record<ActionTrigger, string> = {
  documentScript: "Document script",
  open: "When the file opens",
  beforeClose: "Before it closes",
  beforeSave: "Before saving",
  afterSave: "After saving",
  beforePrint: "Before printing",
  afterPrint: "After printing",
  pageOpen: "When a page opens",
  pageClose: "When a page closes",
  annotation: "From a field or comment",
};

const actionLabels: Record<string, string> = {
  JavaScript: "Runs a script",
  URI: "Opens a website",
  Launch: "Starts another app or file",
  SubmitForm: "Sends form answers",
  ImportData: "Imports form data",
  GoToR: "Opens another PDF",
  GoToE: "Opens an attached PDF",
  ResetForm: "Clears form answers",
  Sound: "Plays a sound",
  Movie: "Plays a video",
  Rendition: "Plays media",
  RichMediaExecute: "Controls embedded media",
  Hide: "Shows or hides content",
  SetOCGState: "Turns layers on or off",
  Named: "Runs a viewer command",
  GoTo: "Goes to a page",
  GoTo3DView: "Changes a 3D view",
  Thread: "Opens an article",
  Trans: "Plays a page transition",
};

const actionItem = ({ trigger, page, name, kinds }: PrivacyReportData["actions"][number]): Item => ({
  key: trigger === "documentScript" && name ? `Document script "${name}"` : page === null ? triggerLabels[trigger] : `${triggerLabels[trigger]}, page ${page}`,
  value: sentence(kinds.map((kind) => actionLabels[kind] ?? `${kind} action`)),
});

const signatureValue = ({ field, signer, signedAt }: PrivacyReportData["signatures"][number]) =>
  `${signer ?? `Field "${field}"`}${signedAt ? `, ${signedAt.slice(0, 10)}` : ""}. Any change to the file removes it.`;

const shortId = (id: string) => id.length > 12 ? `${id.slice(0, 4)}…${id.slice(-4)}` : id;

const categoriesOf = (report: PrivacyReportData): Category[] => {
  const info = (Object.keys(propertyLabels) as (keyof PdfProperties)[])
    .flatMap((key) => {
      const value = report.documentInformation[key];
      return value === null ? [] : [{ key: propertyLabels[key], value: shownValue(key, value) }];
    });
  const xmp: Item[] = report.xmp.summary === null ? [] : [
    ...report.xmp.conflicts.map(({ property, xmp }) => ({ key: `${propertyLabels[property]} differs`, value: `XMP says "${shownValue(property, xmp)}"` })),
    { key: "Stored in XMP", value: report.xmp.summary.properties.join(", ") || "An empty packet" },
  ];
  const { encryption } = report;
  const encryptionItems: Item[] = encryption === null ? [] : [
    encryption.openPassword ? { key: "Open password", value: "Needed to open it" } : { key: "No open password", value: "Anyone can open it" },
    ...(encryption.denied.length ? [{ key: "Not allowed", value: sentence(encryption.denied.map((permission) => permissionLabels[permission])) }] : []),
    ...(encryption.allowed.length ? [{ key: "Allowed", value: sentence(encryption.allowed.map((permission) => permissionLabels[permission])) }] : []),
    { key: "Encryption", value: `${encryption.keyBits ? `${encryption.keyBits}-bit, ` : ""}version ${encryption.version}, revision ${encryption.revision}${encryption.encryptsMetadata ? "" : ", properties not encrypted"}` },
  ];
  const { documentId, producer } = report.identifiers;
  const identifierItems: Item[] = [
    ...(documentId ? [{ key: "Document ID", value: documentId.unchanged ? `${shortId(documentId.permanent)}, unchanged since the first save` : `${shortId(documentId.permanent)}, now ${shortId(documentId.changing)} after later saves` }] : []),
    ...(producer?.version ? [{ key: "App version", value: `${producer.version}, from "${producer.name}"` }] : []),
  ];
  const identifierCount = [documentId ? "1 ID" : null, producer?.version ? "1 app version" : null].filter(Boolean).join(", ");
  const comments = sum(report.annotations);
  const authors = [...new Set(report.annotations.flatMap((group) => group.authors))].sort();
  const commentItems: Item[] = [
    ...(authors.length ? [{ key: "Authors", value: authors.join(", ") }] : []),
    ...report.annotations.map(({ subtype, count, authors }) => ({ key: humanized(subtype), value: `${count}${authors.length ? `, by ${authors.join(", ")}` : ", no author"}` })),
  ];
  const { layers, hidden } = report.optionalContent;
  const offLayers = layers.filter((layer) => !layer.visible).map((layer) => layer.name);
  const onLayers = layers.filter((layer) => layer.visible).map((layer) => layer.name);
  const layerItems: Item[] = [
    ...(offLayers.length ? [{ key: "Off by default", value: offLayers.join(", ") }] : []),
    ...(onLayers.length ? [{ key: "On by default", value: onLayers.join(", ") }] : []),
    ...hidden.map(({ page, subtype, reason }) => ({ key: `Page ${page}`, value: `${humanized(subtype)} ${reason === "hidden" ? "annotation set to hidden" : "annotation placed off the page"}` })),
  ];
  const layerCount = [layers.length ? plural(layers.length, "layer", "layers") : null, offLayers.length ? `${offLayers.length} off` : null, hidden.length ? `${hidden.length} hidden on the page` : null].filter(Boolean).join(", ");
  const links = sum(report.links);
  const { thumbnailPages, pagePieces } = report.leftovers;
  const leftoverItems: Item[] = [
    ...(thumbnailPages.length ? [{ key: "Thumbnails", value: pages(thumbnailPages) }] : []),
    ...pagePieces.map(({ page, applications }) => ({ key: `Page ${page} pieces`, value: `Saved by ${applications.join(", ")}` })),
  ];
  const leftoverCount = [thumbnailPages.length ? plural(thumbnailPages.length, "thumbnail", "thumbnails") : null, pagePieces.length ? plural(pagePieces.length, "page with pieces", "pages with pieces") : null].filter(Boolean).join(", ");
  const updates = report.incrementalUpdates;
  return [
    { name: "Document information", severity: "medium", why: "Anyone you send the file to can see who made it, with which apps, and when.", found: info.length ? { count: plural(info.length, "field", "fields"), items: info } : null },
    { name: "XMP metadata", severity: "medium", why: "A second copy of the properties. It can keep old names after the visible ones change.", found: xmp.length ? { count: plural(xmp.length, "item", "items"), items: xmp } : null },
    { name: "Earlier versions (incremental updates)", severity: "high", why: "Earlier saves are still inside the file. Text you deleted may be recoverable.", found: updates === null ? "unchecked" : updates ? { count: plural(updates, "earlier save", "earlier saves"), items: [{ key: "Earlier saves kept", value: String(updates) }] } : null },
    { name: "Embedded files", severity: "high", why: "Attached files travel with the PDF, even though they aren't on any page.", found: report.embeddedFiles.length ? { count: plural(report.embeddedFiles.length, "file", "files"), items: report.embeddedFiles.map(({ name, size, mimeType, page }) => ({ key: name, value: [size === null ? null : fileSize(size), mimeType, page === null ? "attached to the file" : `attached to page ${page}`].filter(Boolean).join(", ") })) } : null },
    { name: "JavaScript and automatic actions", severity: "high", why: "Scripts and actions can run when the PDF opens, or contact other sites.", found: report.actions.length ? { count: plural(report.actions.length, "action", "actions"), items: report.actions.map(actionItem) } : null },
    { name: "Comments and their authors", severity: "medium", why: "Comments show reviewers' names and can hold notes nobody sees on the page.", found: comments ? { count: `${plural(comments, "comment", "comments")}${authors.length ? `, ${plural(authors.length, "author", "authors")}` : ""}`, items: commentItems } : null },
    { name: "Filled form fields", severity: "low", why: "Typed answers stay in the file, including fields that are hidden or empty-looking.", found: report.filledFormFields.length ? { count: plural(report.filledFormFields.length, "field", "fields"), items: [{ key: "Filled", value: report.filledFormFields.join(", ") }] } : null, pointer: { label: "Open Edit PDF", tool: "pdf-edit" } },
    { name: "Hidden layers (optional content)", severity: "medium", why: "Hidden layers can hold content that doesn't show until someone turns them on.", found: layerItems.length ? { count: layerCount, items: layerItems } : null },
    { name: "Links and the hosts they reach", severity: "low", why: "Opening a link tells that site the file was opened, and from where.", found: links ? { count: `${plural(links, "link", "links")}, ${plural(report.links.length, "host", "hosts")}`, items: [{ key: "Hosts", value: report.links.map(({ host, count }) => count > 1 ? `${host} (${count})` : host).join(", ") }] } : null },
    { name: "Digital signatures", severity: "low", why: "A signature names who signed and when. Changing the file removes it.", found: report.signatures.length ? { count: plural(report.signatures.length, "signature", "signatures"), items: report.signatures.map((signature) => ({ key: "Signed by", value: signatureValue(signature) })) } : null },
    { name: "Encryption and permissions", severity: "low", why: "Shows who can open the file and what readers are allowed to do with it.", found: encryptionItems.length ? { count: encryption?.denied.length ? "Restricted" : "Encrypted", items: encryptionItems } : null },
    { name: "Document IDs", severity: "low", why: "A fixed ID can link this file to other copies or earlier versions of it.", found: identifierItems.length ? { count: identifierCount, items: identifierItems } : null },
    { name: "Thumbnails and page piece data", severity: "medium", why: "Stored thumbnails and page pieces can show content that was later removed.", found: leftoverItems.length ? { count: leftoverCount, items: leftoverItems } : null },
    { name: "Unsupported items", severity: "medium", why: "These parts weren't checked, so the report can't say what they hold.", found: report.unsupported.length ? { count: plural(report.unsupported.length, "item", "items"), items: report.unsupported.map((value, index) => ({ key: `Item ${index + 1}`, value })) } : null },
  ];
};

const parseReport = (outcome: JobOutcome): PrivacyReportData | null => {
  if (outcome.state !== "succeeded") return null;
  try {
    return JSON.parse(outcome.detail) as PrivacyReportData;
  } catch {
    return null;
  }
};

const FileReport = ({ outcome, home, actions }: { outcome: JobOutcome; home: string | null; actions: ReportActions }) => {
  const headingId = useId();
  const report = parseReport(outcome);
  const folder = shortenHomePath(parentFolder(outcome.inputPath), home);
  const failure = report ? null : outcome.state === "succeeded" ? "The report couldn't be read." : rowView(outcome, "Checked").detailLines[0] ?? "This file couldn't be checked.";
  const categories = report ? categoriesOf(report) : [];
  const found = categories.filter((category) => category.found !== null && category.found !== "unchecked");
  const none = categories.filter((category) => category.found === null).length;
  const tally = (["high", "medium", "low"] as const).map((severity) => [severity, found.filter((category) => category.severity === severity).length] as const);
  return (
    <Card role="region" aria-labelledby={headingId} className="privacy-report py-0" data-state={report ? "report" : "failed"}>
      <CardContent className="privacy-report-content p-4">
        <div className="privacy-report-heading">
          <h3 id={headingId}>{fileName(outcome.inputPath)}</h3>
          <span>{report ? `${folder} · ${plural(report.pageCount, "page", "pages")}` : `${folder} · Not read`}</span>
        </div>
        {failure !== null && <p role="alert" className="privacy-report-error">{outcome.state === "skipped" ? "Skipped" : "Failed"}: {failure}</p>}
        {report && (
          <>
            <p className="privacy-report-tally">
              {tally.map(([severity, count]) => <Badge key={severity} variant={severityVariant[severity]}>{count} {severity}</Badge>)}
              <span>{plural(none, "category", "categories")} with none found. {report.unsupported.length === 0 ? "Everything was checked." : report.unsupported.length === 1 ? "1 item wasn't checked." : `${report.unsupported.length} items weren't checked.`}</span>
            </p>
            <ul aria-label="Report categories" className="privacy-report-categories">
              {categories.map((category) => (
                <li key={category.name} data-found={category.found === null ? "false" : category.found === "unchecked" ? "unchecked" : "true"}>
                  {category.found !== null && category.found !== "unchecked" ? (
                    <Card className="privacy-category gap-0 py-0">
                      <CardContent className="privacy-category-content px-4 py-3">
                        <div className="privacy-category-heading">
                          <h4>{category.name}</h4>
                          <Badge variant={severityVariant[category.severity]}>{severityLabel[category.severity]}</Badge>
                          <span>{category.found.count}</span>
                          {category.pointer && actions.canOpen(category.pointer.tool) && (
                            <Button variant="outline" size="sm" className="privacy-category-pointer" onClick={() => actions.open(category.pointer!.tool, outcome.inputPath)}>{category.pointer.label}</Button>
                          )}
                        </div>
                        <p className="privacy-category-why"><strong>Why it matters: </strong>{category.why}</p>
                        <dl className="privacy-category-items">
                          {category.found.items.map((item, index) => (
                            <div key={index}>
                              <dt>{item.key}</dt>
                              <dd>{item.value}</dd>
                            </div>
                          ))}
                        </dl>
                      </CardContent>
                    </Card>
                  ) : (
                    <div className="privacy-category-heading privacy-category-none">
                      <h4>{category.name}</h4>
                      <span>{category.found === "unchecked" ? "Couldn't be read. See Unsupported items." : "None found"}</span>
                    </div>
                  )}
                </li>
              ))}
            </ul>
          </>
        )}
      </CardContent>
    </Card>
  );
};

export const PrivacyReports = ({ results, actions }: { results: JobOutcome[]; actions: ReportActions }) => {
  const home = useHomeDir();
  const failed = results.filter((outcome) => parseReport(outcome) === null).length;
  return (
    <section className="privacy-reports" aria-label="Privacy reports">
      <p role="status" className="result-summary">
        <strong>Done. {plural(results.length - failed, "report", "reports")}{failed ? `, ${failed} failed` : ""}</strong>
        <span className="result-summary-hint">Nothing was changed in any PDF.</span>
      </p>
      {results.map((outcome) => <FileReport key={outcome.inputPath} outcome={outcome} home={home} actions={actions} />)}
    </section>
  );
};

export const privacyCheckTool: PdfEditorBatchTool = {
  description: "One report per PDF in 14 categories, from document information to unsupported items. Each category shows what was found and why it matters, or says None found.",
  request: (paths) => ({ paths }),
  report: {
    runLabel: (count) => count ? `Check ${plural(count, "PDF", "PDFs")}` : "Check PDFs",
    note: "Reads each PDF and makes a report. No file is changed or saved.",
    render: (results, actions) => <PrivacyReports results={results} actions={actions} />,
  },
};
