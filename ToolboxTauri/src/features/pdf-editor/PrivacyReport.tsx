import { useId } from "react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import type { JobOutcome } from "../../contracts";
import { shortenHomePath, useHomeDir } from "../../homePath";
import { parentFolder } from "../../components/intake";
import { fileName, rowView } from "../../components/resultRow";
import type { PdfEditorBatchTool } from "./batchTools";

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
  unsupported: string[];
}

type Severity = "high" | "medium" | "low";
type Item = { key: string; value: string };
type Category = { name: string; severity: Severity; why: string; found: { count: string; items: Item[] } | null };

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
  return [
    { name: "Document information", severity: "medium", why: "Anyone you send the file to can see who made it, with which apps, and when.", found: info.length ? { count: plural(info.length, "field", "fields"), items: info } : null },
    { name: "XMP metadata", severity: "medium", why: "A second copy of the properties. It can keep old names after the visible ones change.", found: xmp.length ? { count: plural(xmp.length, "item", "items"), items: xmp } : null },
    { name: "Encryption and permissions", severity: "low", why: "Shows who can open the file and what readers are allowed to do with it.", found: encryptionItems.length ? { count: encryption?.denied.length ? "Restricted" : "Encrypted", items: encryptionItems } : null },
    { name: "Document IDs", severity: "low", why: "A fixed ID can link this file to other copies or earlier versions of it.", found: identifierItems.length ? { count: identifierCount, items: identifierItems } : null },
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

const FileReport = ({ outcome, home }: { outcome: JobOutcome; home: string | null }) => {
  const headingId = useId();
  const report = parseReport(outcome);
  const folder = shortenHomePath(parentFolder(outcome.inputPath), home);
  const failure = report ? null : outcome.state === "succeeded" ? "The report couldn't be read." : rowView(outcome, "Checked").detailLines[0] ?? "This file couldn't be checked.";
  const categories = report ? categoriesOf(report) : [];
  const found = categories.filter((category) => category.found);
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
              <span>{plural(categories.length - found.length, "category", "categories")} with none found. {report.unsupported.length === 0 ? "Everything was checked." : report.unsupported.length === 1 ? "1 item wasn't checked." : `${report.unsupported.length} items weren't checked.`}</span>
            </p>
            <ul aria-label="Report categories" className="privacy-report-categories">
              {categories.map((category) => (
                <li key={category.name} data-found={category.found ? "true" : "false"}>
                  {category.found ? (
                    <Card className="privacy-category gap-0 py-0">
                      <CardContent className="privacy-category-content px-4 py-3">
                        <div className="privacy-category-heading">
                          <h4>{category.name}</h4>
                          <Badge variant={severityVariant[category.severity]}>{severityLabel[category.severity]}</Badge>
                          <span>{category.found.count}</span>
                        </div>
                        <p className="privacy-category-why"><strong>Why it matters: </strong>{category.why}</p>
                        <dl className="privacy-category-items">
                          {category.found.items.map((item) => (
                            <div key={item.key}>
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
                      <span>None found</span>
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

export const PrivacyReports = ({ results }: { results: JobOutcome[] }) => {
  const home = useHomeDir();
  const failed = results.filter((outcome) => parseReport(outcome) === null).length;
  return (
    <section className="privacy-reports" aria-label="Privacy reports">
      <p role="status" className="result-summary">
        <strong>Done. {plural(results.length - failed, "report", "reports")}{failed ? `, ${failed} failed` : ""}</strong>
        <span className="result-summary-hint">Nothing was changed in any PDF.</span>
      </p>
      {results.map((outcome) => <FileReport key={outcome.inputPath} outcome={outcome} home={home} />)}
    </section>
  );
};

export const privacyCheckTool: PdfEditorBatchTool = {
  description: "One report per PDF: document information, XMP metadata, encryption and permissions, document IDs, and anything that couldn't be checked. Each category shows what was found and why it matters, or says None found.",
  request: (paths) => ({ paths }),
  report: {
    runLabel: (count) => count ? `Check ${plural(count, "PDF", "PDFs")}` : "Check PDFs",
    note: "Reads each PDF and makes a report. No file is changed or saved.",
    render: (results) => <PrivacyReports results={results} />,
  },
};
