import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";

const count = (n: number, noun: string) => `${n} ${noun}${n === 1 ? "" : "s"}`;

export const PdfToEpubView = ({ utility, initialPaths = [], onFilesChange }: { utility: ToolDefinition; initialPaths?: readonly string[]; onFilesChange?: (paths: readonly string[]) => void }) => {
  const [removeHeadersAndFooters, setRemoveHeadersAndFooters] = useState(true);
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
      onRun={(paths, outputLocation, { runId }) => invoke<ToolResult>("pdf_to_epub", { runId, request: { paths, outputLocation, removeHeadersAndFooters } })}
    >
      {({ files, run, loading, destination }) => {
        const runLabel = files.length === 0 ? "Convert" : `Convert ${count(files.length, "PDF")}`;
        const blocker = files.length === 0 ? "Add PDFs to start" : null;
        return (
          <Card className="workspace-control-panel py-0">
            <CardContent className="workspace-control-panel-content grid gap-4 p-4">
              <div>
                <h2 className="workspace-active-command">{utility.title}</h2>
                <p className="workspace-panel-copy">{utility.blurb}</p>
              </div>
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
