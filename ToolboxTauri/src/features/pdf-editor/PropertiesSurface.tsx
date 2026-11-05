import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../../components/ToolScaffold";
import { ShortcutHint } from "../../components/ShortcutHint";
import { shortcutTarget } from "../../shortcuts";
import type { JobOutcome, ToolDefinition, ToolResult } from "../../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { fileName } from "../../components/resultRow";

type Field = "title" | "author" | "subject";
type Action = "keep" | "set" | "clear";
type Choice = { action: Action; value: string };
type Current = { title: string | null; author: string | null; subject: string | null; modificationDate: string | null };
type Read = { state: "read"; values: Current } | { state: "locked" } | { state: "unreadable" };

const PROTECTED = "Edit Properties can't edit a password-protected PDF yet. Nothing was saved.";
const NONE = "(none)";
const fields: { id: Field; label: string; noun: string; article: string }[] = [
  { id: "title", label: "Title", noun: "title", article: "a" },
  { id: "author", label: "Author", noun: "author", article: "an" },
  { id: "subject", label: "Subject", noun: "subject", article: "a" },
];
const keepAll: Record<Field, Choice> = { title: { action: "keep", value: "" }, author: { action: "keep", value: "" }, subject: { action: "keep", value: "" } };

const filesHave = (count: number) => `${count} ${count === 1 ? "file has" : "files have"}`;
const joined = (parts: string[]) => parts.length < 2 ? parts.join("") : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;

const nowHint = (field: (typeof fields)[number], files: string[], reads: Map<string, Read>, keeping: boolean) => {
  if (files.length === 0) return "Add PDFs to see the current value.";
  const known = files.map((path) => reads.get(path)).filter((read): read is Extract<Read, { state: "read" }> => read?.state === "read");
  if (known.length === 0) return null;
  if (files.length === 1) return `Now: ${known[0].values[field.id] ?? NONE}`;
  const values = known.map(({ values }) => values[field.id]).filter((value): value is string => value !== null);
  const missing = known.length - values.length;
  const distinct = new Set(values).size;
  const parts = field.id === "subject"
    ? [values.length ? `${filesHave(values.length)} ${field.article} ${field.noun}` : null, missing ? (values.length ? `${missing} ${missing === 1 ? "has" : "have"} none` : `${filesHave(missing)} none`) : null]
    : [distinct > 1 ? `${distinct} different ${field.noun}s` : values.length === 1 ? `1 file has ${field.article} ${field.noun}` : values.length ? `${values.length} files have the same ${field.noun}` : null, missing ? `${filesHave(missing)} none` : null];
  const now = `Now: ${parts.filter(Boolean).join(", ")}`;
  return keeping ? `${now}. Each keeps its own.` : now;
};

const fieldNote = (field: (typeof fields)[number], choice: Choice, count: number) => {
  if (count === 0) return null;
  if (choice.action === "clear") return count === 1 ? "Removed from the file, including its XMP copy." : `Removed from all ${count} files, including their XMP copies.`;
  if (choice.action === "set" && count > 1) return `Replaces the ${field.noun} in all ${count} files.`;
  return null;
};

const describe = (choices: Record<Field, Choice>, count: number) => {
  const nouns = (action: Action) => fields.filter(({ id }) => choices[id].action === action).map(({ noun }) => noun);
  const changes = [nouns("set").length ? `sets the ${joined(nouns("set"))}` : null, nouns("clear").length ? `clears the ${joined(nouns("clear"))}` : null].filter((part): part is string => part !== null);
  const sentence = joined(changes);
  return `${sentence.charAt(0).toUpperCase()}${sentence.slice(1)}. ${count === 1 ? "Everything else is kept." : "A file that already matches is skipped."}`;
};

const reasonFor = (choices: Record<Field, Choice>, count: number) => {
  if (count === 0) return "Add PDFs to edit.";
  if (fields.every(({ id }) => choices[id].action === "keep")) return "Every field is set to Keep, so nothing would change. Choose Set or Clear for a field.";
  const empty = fields.find(({ id }) => choices[id].action === "set" && choices[id].value.trim() === "");
  if (empty) return `Enter ${empty.article} ${empty.noun}, or choose Keep or Clear.`;
  return null;
};

const runLabel = (count: number) => count === 0 ? "Save edited copies" : count === 1 ? "Save edited copy" : `Save ${count} edited copies`;

const patchFor = (choices: Record<Field, Choice>) => Object.fromEntries(fields.map(({ id }) => [id, choices[id].action === "set" ? { action: "set", value: choices[id].value } : { action: choices[id].action }]));

/** Reads each added PDF's current title, author, subject and modified date once. */
const useCurrentValues = (files: readonly string[]) => {
  const [reads, setReads] = useState(() => new Map<string, Read>());
  const requested = useRef(new Set<string>());
  useEffect(() => {
    const paths = files.filter((path) => !requested.current.has(path));
    if (paths.length === 0) return;
    paths.forEach((path) => requested.current.add(path));
    void invoke<JobOutcome[]>("read_pdf_properties", { request: { paths } }).then((outcomes) => {
      setReads((previous) => {
        const next = new Map(previous);
        for (const outcome of outcomes) {
          next.set(outcome.inputPath, outcome.state === "succeeded"
            ? { state: "read", values: JSON.parse(outcome.detail) as Current }
            : outcome.failure?.message === PROTECTED ? { state: "locked" } : { state: "unreadable" });
        }
        return next;
      });
    }, () => paths.forEach((path) => requested.current.delete(path)));
  }, [files]);
  return reads;
};

const CurrentValues = ({ read }: { read: Read | undefined }) => {
  if (read?.state === "locked") return <span className="pdf-properties-now"><span className="pdf-properties-cell" data-empty="true">Locked</span></span>;
  if (read?.state !== "read") return null;
  const { title, author, modificationDate } = read.values;
  const cells: [string, string | null][] = [["Title now", title], ["Author now", author], ["Modified now", modificationDate && modificationDate.slice(0, 10)]];
  return (
    <span className="pdf-properties-now">
      {cells.map(([label, value]) => (
        <span key={label} className="pdf-properties-cell" aria-label={`${label}: ${value ?? NONE}`} title={value ?? NONE} data-empty={value === null || undefined}>{value ?? NONE}</span>
      ))}
    </span>
  );
};

export const PdfPropertiesSurface = ({ utility, initialPaths, onFilesChange }: {
  utility: ToolDefinition;
  initialPaths?: readonly string[];
  onFilesChange?: (paths: readonly string[]) => void;
}) => {
  const [files, setFiles] = useState<readonly string[]>(initialPaths ?? []);
  const [choices, setChoices] = useState(keepAll);
  const reads = useCurrentValues(files);
  const filesChanged = useCallback((paths: readonly string[]) => {
    setFiles(paths);
    onFilesChange?.(paths);
  }, [onFilesChange]);
  const choose = (field: Field, choice: Partial<Choice>) => setChoices((previous) => ({ ...previous, [field]: { ...previous[field], ...choice } }));

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="pdf-editor-batch"
      utility={utility}
      initialPaths={initialPaths}
      onFilesChange={filesChanged}
      onRun={(paths, outputLocation, { runId }) => invoke<ToolResult>(utility.command, { runId, request: { paths, patch: patchFor(choices), outputLocation } })}
      renderFileExtra={(path) => <CurrentValues read={reads.get(path)} />}
    >
      {({ files, run, loading, destination }) => {
        const reason = reasonFor(choices, files.length);
        return (
          <Card className="workspace-control-panel py-0">
            <CardContent className="workspace-control-panel-content grid gap-4 p-4">
              <div>
                <h2 className="workspace-active-command">{utility.title}</h2>
                {files.length > 0 && (
                  <p className="workspace-panel-copy">
                    {files.length === 1
                      ? `Current values of ${fileName(files[0])} are shown under each field.`
                      : `Changes apply to all ${files.length} PDFs. Keep leaves each file's own value, even where files differ.`}
                  </p>
                )}
              </div>
              {fields.map((field) => {
                const choice = choices[field.id];
                const labelId = `pdf-properties-${field.id}`;
                const hint = nowHint(field, files, reads, choice.action === "keep" && files.length > 1);
                const note = fieldNote(field, choice, files.length);
                return (
                  <div key={field.id} role="group" aria-labelledby={labelId} className="pdf-properties-field grid gap-1.5">
                    <div className="flex items-center gap-2">
                      <span id={labelId} className="workspace-panel-label flex-1">{field.label}</span>
                      <ToggleGroup
                        type="single"
                        variant="outline"
                        size="sm"
                        aria-label={field.label}
                        value={choice.action}
                        disabled={loading}
                        onValueChange={(action) => action && choose(field.id, { action: action as Action })}
                      >
                        <ToggleGroupItem value="keep">Keep</ToggleGroupItem>
                        <ToggleGroupItem value="set">Set</ToggleGroupItem>
                        <ToggleGroupItem value="clear">Clear</ToggleGroupItem>
                      </ToggleGroup>
                    </div>
                    {hint && <p className="workspace-note">{hint}</p>}
                    {choice.action === "set" && (
                      <Input
                        aria-label={`New ${field.label}`}
                        placeholder={field.label}
                        value={choice.value}
                        disabled={loading}
                        onChange={(event) => choose(field.id, { value: event.target.value })}
                      />
                    )}
                    {note && <p className="workspace-note">{note}</p>}
                  </div>
                );
              })}
              {destination}
              <p className="workspace-note" data-testid="pdf-properties-reason">{reason ?? describe(choices, files.length)}</p>
              <Button
                variant="default"
                disabled={loading || reason !== null || utility.capability.nativeAvailability !== "available"}
                onClick={run}
                className="workspace-primary-action"
                {...shortcutTarget("run")}
              >
                {runLabel(files.length)}
                <ShortcutHint id="run" />
              </Button>
            </CardContent>
          </Card>
        );
      }}
    </ToolScaffold>
  );
};
