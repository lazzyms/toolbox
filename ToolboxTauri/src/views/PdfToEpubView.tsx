import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { JobOutcome, ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";
import { parsePageRanges } from "../shared/pageRanges";

const count = (n: number, noun: string) => `${n} ${noun}${n === 1 ? "" : "s"}`;

type Cover = "firstPage" | "none";
type PageScope = "all" | "range";
/** The title and author shown for the one PDF being converted; `edited` stops a late read from replacing what was typed. */
type BookDetails = { path: string; title: string; author: string; edited: boolean };

/** Fills in the one PDF's title and author from its properties, once per PDF. */
const useBookDetails = (path: string | null, setDetails: (update: (current: BookDetails | null) => BookDetails | null) => void) => {
  useEffect(() => {
    if (path === null) return;
    setDetails((current) => current?.path === path ? current : { path, title: "", author: "", edited: false });
    let current = true;
    void invoke<JobOutcome[]>("read_pdf_properties", { request: { paths: [path] } }).then(([outcome]) => {
      if (!current || outcome?.state !== "succeeded") return;
      const { title, author } = JSON.parse(outcome.detail) as { title?: string | null; author?: string | null };
      setDetails((details) => details?.path === path && !details.edited ? { ...details, title: title ?? "", author: author ?? "" } : details);
    }, () => undefined);
    return () => { current = false; };
  }, [path, setDetails]);
};

const BookDetailsFields = ({ files, details, setDetails, disabled }: {
  files: readonly string[];
  details: BookDetails | null;
  setDetails: (update: (current: BookDetails | null) => BookDetails | null) => void;
  disabled: boolean;
}) => {
  const single = files.length === 1 ? files[0] : null;
  useBookDetails(single, setDetails);
  const edit = (field: "title" | "author", value: string) =>
    setDetails((current) => current && current.path === single ? { ...current, [field]: value, edited: true } : current);
  const shown = single !== null && details?.path === single ? details : null;
  return (
    <fieldset className="workspace-field" disabled={disabled}>
      <legend className="workspace-panel-label">Book details</legend>
      {single === null ? (
        <p className="workspace-note">Each book uses its PDF's title and author. Convert one PDF to edit them.</p>
      ) : (
        <>
          <div className="workspace-field">
            <Label htmlFor="epub-title">Title</Label>
            <Input id="epub-title" aria-describedby="epub-details-hint" value={shown?.title ?? ""} onChange={(event) => edit("title", event.target.value)} />
          </div>
          <div className="workspace-field">
            <Label htmlFor="epub-author">Author</Label>
            <Input id="epub-author" aria-describedby="epub-details-hint" value={shown?.author ?? ""} onChange={(event) => edit("author", event.target.value)} />
          </div>
          <p id="epub-details-hint" className="workspace-note">Filled in from the PDF. Your changes go into this EPUB only; the PDF isn't changed.</p>
        </>
      )}
    </fieldset>
  );
};

export const PdfToEpubView = ({ utility, initialPaths = [], onFilesChange }: { utility: ToolDefinition; initialPaths?: readonly string[]; onFilesChange?: (paths: readonly string[]) => void }) => {
  const [removeHeadersAndFooters, setRemoveHeadersAndFooters] = useState(true);
  const [cover, setCover] = useState<Cover>("firstPage");
  const [details, setDetails] = useState<BookDetails | null>(null);
  const [pageScope, setPageScope] = useState<PageScope>("all");
  const [pageRange, setPageRange] = useState("");
  const parsedRange = pageScope === "range" ? parsePageRanges(pageRange) : null;
  const rangeIssue = parsedRange && !parsedRange.ok ? parsedRange.message : null;
  const overrides = (paths: readonly string[]) =>
    paths.length === 1 && details?.path === paths[0] ? { title: details.title, author: details.author } : {};
  return (
    <ToolScaffold
      key={initialPaths.join("\0") || "no-initial-pdf"}
      variant="workspace"
      sessionKey="pdf-convert"
      utility={utility}
      initialPaths={initialPaths}
      onFilesChange={onFilesChange}
      emptyHint="Accepts PDF. Makes one EPUB for each PDF. Works best on text-heavy books, reports and articles."
      exampleHint="An existing name is never replaced; the new one gets -1, -2."
      destinationMentionsSettingKept
      onRun={(paths, outputLocation, { runId }) => invoke<ToolResult>("pdf_to_epub", { runId, request: {
        paths,
        outputLocation,
        removeHeadersAndFooters,
        cover,
        pageRange: pageScope === "range" ? pageRange.trim() : null,
        ...overrides(paths),
      } })}
    >
      {({ files, run, loading, destination }) => {
        const runLabel = files.length === 0 ? "Convert" : `Convert ${count(files.length, "PDF")}`;
        const blocker = files.length === 0 ? "Add PDFs to start" : rangeIssue ? "Fix the page range first" : null;
        return (
          <Card className="workspace-control-panel py-0">
            <CardContent className="workspace-control-panel-content grid gap-4 p-4">
              <div>
                <h2 className="workspace-active-command">{utility.title}</h2>
                <p className="workspace-panel-copy">{utility.blurb}</p>
              </div>
              <fieldset className="workspace-field" disabled={loading}>
                <legend className="workspace-panel-label">Pages</legend>
                <ToggleGroup
                  type="single"
                  variant="outline"
                  size="sm"
                  aria-label="Pages"
                  value={pageScope}
                  disabled={loading}
                  onValueChange={(value) => { if (value) setPageScope(value as PageScope); }}
                >
                  <ToggleGroupItem value="all">All pages</ToggleGroupItem>
                  <ToggleGroupItem value="range">Range</ToggleGroupItem>
                </ToggleGroup>
                {pageScope === "range" && (
                  <>
                    <Label htmlFor="epub-page-range" className="sr-only">Page range for every PDF</Label>
                    <Input
                      id="epub-page-range"
                      value={pageRange}
                      placeholder="1-3, 5, 8-"
                      aria-invalid={rangeIssue !== null && pageRange.trim() !== ""}
                      aria-describedby={rangeIssue && pageRange.trim() ? "epub-range-error epub-range-hint" : "epub-range-hint epub-range-scope"}
                      onChange={(event) => setPageRange(event.target.value)}
                    />
                    {rangeIssue && pageRange.trim() && <p id="epub-range-error" className="workspace-note" role="alert">{rangeIssue}</p>}
                    <p id="epub-range-hint" className="workspace-note">Commas and dashes, like 1-3, 5, 8-. 8- means page 8 to the end.</p>
                  </>
                )}
                <p id="epub-range-scope" className="workspace-note">
                  {pageScope === "range"
                    ? "One range applies to every PDF. Pages past a PDF's last page are left out, and a PDF with no pages left is skipped."
                    : "Every page of every PDF."}
                </p>
              </fieldset>
              <fieldset className="workspace-field" disabled={loading}>
                <legend className="workspace-panel-label">Clean up</legend>
                <Label className="workspace-check" htmlFor="epub-remove-running-text">
                  <Checkbox
                    id="epub-remove-running-text"
                    aria-describedby="epub-remove-running-text-hint"
                    checked={removeHeadersAndFooters}
                    disabled={loading}
                    onCheckedChange={(checked) => setRemoveHeadersAndFooters(checked === true)}
                  />
                  <span>Remove running headers, footers and page numbers</span>
                </Label>
                <p id="epub-remove-running-text-hint" className="workspace-note">Text repeated on most pages is left out of the book.</p>
              </fieldset>
              <fieldset className="workspace-field" disabled={loading}>
                <legend className="workspace-panel-label">Cover</legend>
                <ToggleGroup
                  type="single"
                  variant="outline"
                  size="sm"
                  aria-label="Cover"
                  aria-describedby="epub-cover-hint"
                  value={cover}
                  disabled={loading}
                  onValueChange={(value) => { if (value) setCover(value as Cover); }}
                >
                  <ToggleGroupItem value="firstPage">First page</ToggleGroupItem>
                  <ToggleGroupItem value="none">No cover</ToggleGroupItem>
                </ToggleGroup>
                <p id="epub-cover-hint" className="workspace-note">The first page in the range becomes the cover.</p>
              </fieldset>
              <BookDetailsFields files={files} details={details} setDetails={setDetails} disabled={loading} />
              {destination}
              {files.length > 0 && (
                <p className="workspace-note">
                  This will make up to {count(files.length, "EPUB")}. A PDF with no selectable text is skipped.
                </p>
              )}
              <p className="workspace-note">Originals are only read, never changed.</p>
              {blocker && <p id="epub-run-blocker" className="workspace-note">{blocker}</p>}
              <Button
                variant="default"
                aria-label={runLabel}
                aria-describedby={blocker ? "epub-run-blocker" : undefined}
                disabled={loading || blocker !== null}
                onClick={run}
                className="workspace-primary-action"
                {...shortcutTarget("run")}
              >
                {runLabel}
                <ShortcutHint id="run" />
              </Button>
            </CardContent>
          </Card>
        );
      }}
    </ToolScaffold>
  );
};
