import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CircleAlert } from "lucide-react";
import { ToolScaffold } from "../../components/ToolScaffold";
import { ShortcutHint } from "../../components/ShortcutHint";
import { shortcutTarget } from "../../shortcuts";
import type { JobOutcome, PasswordSet, ToolDefinition, ToolResult } from "../../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { fileName } from "../../components/resultRow";
import { BatchPasswordFields, FilePasswordCell, ForgetClosedFiles, passwordsFor, usePasswordSet } from "../../components/PasswordFields";

type Field = "title" | "author" | "subject" | "creationDate" | "modificationDate";
type Action = "keep" | "set" | "clear";
type Choice = { action: Action; value: string };
type Current = Record<Field, string | null>;
type Read = { state: "read"; values: Current } | { state: "locked" } | { state: "unreadable" };
type FieldSpec = { id: Field; label: string; noun: string; article: string; summaryNoun: string; date?: true };

const LOCKED_NOTE = "Needs an open password. The one for all files is tried.";
const RETYPE_DELAY_MS = 400;
const NONE = "(none)";
const DATE_PLACEHOLDER = "YYYY-MM-DD HH:MM, your local time";
const DATE_FORMAT = "Use YYYY-MM-DD HH:MM, like 2026-02-28 09:00.";
const fields: FieldSpec[] = [
  { id: "title", label: "Title", noun: "title", article: "a", summaryNoun: "title" },
  { id: "author", label: "Author", noun: "author", article: "an", summaryNoun: "author" },
  { id: "subject", label: "Subject", noun: "subject", article: "a", summaryNoun: "subject" },
  { id: "creationDate", label: "Created", noun: "creation date", article: "a", summaryNoun: "date", date: true },
  { id: "modificationDate", label: "Modified", noun: "modification date", article: "a", summaryNoun: "date", date: true },
];
const keepAll = Object.fromEntries(fields.map(({ id }) => [id, { action: "keep", value: "" }])) as Record<Field, Choice>;

const pad = (value: number, length = 2) => String(Math.abs(value)).padStart(length, "0");

/** Turns "YYYY-MM-DD HH:MM" in the viewer's time zone into a dated moment with its offset, or explains what is wrong. */
const parseLocalDate = (text: string): { ok: true; value: string } | { ok: false; error: string } => {
  const match = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2})$/.exec(text.trim());
  if (!match) return { ok: false, error: DATE_FORMAT };
  const [year, month, day, hour, minute] = match.slice(1).map(Number);
  const calendar = new Date(Date.UTC(year, month - 1, day));
  if (calendar.getUTCFullYear() !== year || calendar.getUTCMonth() !== month - 1 || calendar.getUTCDate() !== day) return { ok: false, error: `${match[1]}-${match[2]}-${match[3]} isn't a real date. ${DATE_FORMAT}` };
  if (hour > 23 || minute > 59) return { ok: false, error: `${match[4]}:${match[5]} isn't a real time. ${DATE_FORMAT}` };
  const local = new Date(year, month - 1, day, hour, minute);
  local.setFullYear(year);
  const offset = -local.getTimezoneOffset();
  return { ok: true, value: `${match[1]}-${match[2]}-${match[3]}T${match[4]}:${match[5]}:00${offset < 0 ? "-" : "+"}${pad(Math.trunc(offset / 60))}:${pad(offset % 60)}` };
};

const shownValue = (field: FieldSpec, value: string | null) => value !== null && field.date ? value.slice(0, 16).replace("T", " ") : value;
const dateError = (field: FieldSpec, choice: Choice) => {
  if (!field.date || choice.action !== "set" || choice.value.trim() === "") return null;
  const parsed = parseLocalDate(choice.value);
  return parsed.ok ? null : parsed.error;
};

const filesHave = (count: number) => `${count} ${count === 1 ? "file has" : "files have"}`;
const joined = (parts: string[]) => parts.length < 2 ? parts.join("") : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;

const nowHint = (field: FieldSpec, files: readonly string[], reads: Map<string, Read>, keeping: boolean) => {
  if (files.length === 0) return "Add PDFs to see the current value.";
  const known = files.map((path) => reads.get(path)).filter((read): read is Extract<Read, { state: "read" }> => read?.state === "read");
  if (known.length === 0) return null;
  if (files.length === 1) return `Now: ${shownValue(field, known[0].values[field.id]) ?? NONE}`;
  const values = known.map(({ values }) => values[field.id] ?? null).filter((value): value is string => value !== null);
  const missing = known.length - values.length;
  const distinct = new Set(values).size;
  const parts = field.id === "subject"
    ? [values.length ? `${filesHave(values.length)} ${field.article} ${field.noun}` : null, missing ? (values.length ? `${missing} ${missing === 1 ? "has" : "have"} none` : `${filesHave(missing)} none`) : null]
    : [distinct > 1 ? `${distinct} different ${field.summaryNoun}s` : values.length === 1 ? `1 file has ${field.article} ${field.summaryNoun}` : values.length ? `${values.length} files have the same ${field.summaryNoun}` : null, missing ? `${filesHave(missing)} none` : null];
  const now = `Now: ${parts.filter(Boolean).join(", ")}`;
  return keeping ? `${now}. Each keeps its own.` : now;
};

const fieldNote = (field: FieldSpec, choice: Choice, count: number) => {
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
  const invalid = fields.find((field) => dateError(field, choices[field.id]) !== null);
  if (invalid) return `Fix the ${invalid.noun} to save.`;
  return null;
};

const runLabel = (count: number) => count === 0 ? "Save edited copies" : count === 1 ? "Save edited copy" : `Save ${count} edited copies`;

const setValue = (field: FieldSpec, value: string) => {
  if (!field.date) return value;
  const parsed = parseLocalDate(value);
  return parsed.ok ? parsed.value : value;
};
const patchFor = (choices: Record<Field, Choice>) => Object.fromEntries(fields.map((field) => {
  const choice = choices[field.id];
  return [field.id, choice.action === "set" ? { action: "set", value: setValue(field, choice.value) } : { action: choice.action }];
}));

const passwordFor = (path: string, passwords: PasswordSet) => passwords.overrides[path] ?? passwords.batch;

/** Reads each added PDF's current title, author, subject and dates once, and again for a locked one when the password it would be opened with changes. */
const useCurrentValues = (files: readonly string[], passwords: PasswordSet) => {
  const [reads, setReads] = useState(() => new Map<string, Read>());
  const tried = useRef(new Map<string, string>());
  const readsRef = useRef(reads);
  readsRef.current = reads;
  useEffect(() => {
    const open = new Set(files);
    for (const path of tried.current.keys()) if (!open.has(path)) tried.current.delete(path);
    const unread = files.filter((path) => !tried.current.has(path));
    const retry = files.filter((path) => readsRef.current.get(path)?.state === "locked" && tried.current.get(path) !== passwordFor(path, passwords));
    const read = (paths: string[]) => {
      if (paths.length === 0) return;
      const sent = new Map(paths.map((path) => [path, passwordFor(path, passwords)]));
      sent.forEach((password, path) => tried.current.set(path, password));
      void invoke<JobOutcome[]>("read_pdf_properties", { request: { paths, ...passwordsFor(paths, passwords) } }).then((outcomes) => {
        const current = outcomes.filter((outcome) => tried.current.get(outcome.inputPath) === sent.get(outcome.inputPath));
        if (current.length === 0) return;
        setReads((previous) => {
          const next = new Map(previous);
          for (const outcome of current) {
            next.set(outcome.inputPath, outcome.state === "succeeded"
              ? { state: "read", values: JSON.parse(outcome.detail) as Current }
              : outcome.failure?.kind === "wrongPassword" ? { state: "locked" } : { state: "unreadable" });
          }
          return next;
        });
      }, () => paths.forEach((path) => { if (tried.current.get(path) === sent.get(path)) tried.current.delete(path); }));
    };
    read(unread);
    if (retry.length === 0) return;
    const timer = window.setTimeout(() => read(retry), RETYPE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [files, passwords.batch, passwords.overrides]);
  return reads;
};

const CurrentValues = ({ read }: { read: Read | undefined }) => {
  if (read?.state === "locked") return (
    <span className="pdf-properties-now">
      <span className="pdf-properties-cell" data-empty="true">Locked</span>
      <span className="pdf-properties-cell pdf-properties-locked" title={LOCKED_NOTE}>{LOCKED_NOTE}</span>
    </span>
  );
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
  const passwords = usePasswordSet();
  const reads = useCurrentValues(files, passwords.set);
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
      onRun={(paths, outputLocation, { runId }) => invoke<ToolResult>(utility.command, { runId, request: { paths, ...passwordsFor(paths, passwords.set), patch: patchFor(choices), outputLocation } })}
      retryWithPassword={(inputPath, password) => (paths, outputLocation, { runId }) =>
        invoke<ToolResult>(utility.command, { runId, request: { paths, ...passwordsFor(paths, { batch: "", overrides: { [inputPath]: password } }), patch: patchFor(choices), outputLocation } })}
      renderFileExtra={(path, selected, locked) => (
        <>
          <CurrentValues read={reads.get(path)} />
          <FilePasswordCell mode="open" path={path} passwords={passwords} selected={selected} disabled={locked} />
        </>
      )}
    >
      {({ files, run, loading, destination }) => {
        const reason = reasonFor(choices, files.length);
        const needsPassword = files.length > 1 || files.some((path) => reads.get(path)?.state === "locked");
        return (
          <Card className="workspace-control-panel py-0">
            <ForgetClosedFiles files={files} passwords={passwords} />
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
                const error = dateError(field, choice);
                const errorId = `${labelId}-error`;
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
                        placeholder={field.date ? DATE_PLACEHOLDER : field.label}
                        value={choice.value}
                        disabled={loading}
                        aria-invalid={error !== null || undefined}
                        aria-describedby={error !== null ? errorId : undefined}
                        onChange={(event) => choose(field.id, { value: event.target.value })}
                      />
                    )}
                    {error !== null && <p id={errorId} role="alert" className="password-error"><CircleAlert aria-hidden="true" />{error}</p>}
                    {note && <p className="workspace-note">{note}</p>}
                  </div>
                );
              })}
              {needsPassword && <BatchPasswordFields mode="open" passwords={passwords} files={files} />}
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
