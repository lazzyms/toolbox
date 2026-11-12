import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { JobOutcome, ToolDefinition, ToolResult } from "../contracts";
import {
    assignBatesRanges,
    batesFormatIssue,
    batesIdentifier,
    batesPositions,
    batesRangeLabel,
    defaultBatesFormat,
    type BatesFormat,
    type BatesPosition,
} from "../features/pdf-editor/bates";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";

const textFonts = ["Noto Sans", "Noto Serif", "Noto Sans Mono"];
const UNUSED = " unused";

/** A page count per path: a number once read, null when it couldn't be read, absent while it is being read. */
type PageCounts = ReadonlyMap<string, number | null>;

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** The numbers a finished run reported for a file: the range it was stamped with, or the range a failure left unused. */
const reportedNumbers = (outcome: JobOutcome) => {
    if (!outcome.detail) return null;
    return outcome.detail.endsWith(UNUSED)
        ? { range: outcome.detail.slice(0, -UNUSED.length), unused: true }
        : { range: outcome.detail, unused: false };
};

export const BatesView = ({ utility, initialPaths = [], onFilesChange }: { utility: ToolDefinition; initialPaths?: readonly string[]; onFilesChange?: (paths: readonly string[]) => void }) => {
    const [prefix, setPrefix] = useState(defaultBatesFormat.prefix);
    const [suffix, setSuffix] = useState(defaultBatesFormat.suffix);
    const [start, setStart] = useState(String(defaultBatesFormat.start));
    const [digits, setDigits] = useState(String(defaultBatesFormat.digits));
    const [position, setPosition] = useState<BatesPosition>(defaultBatesFormat.position);
    const [fontFamily, setFontFamily] = useState(defaultBatesFormat.fontFamily);
    const [fontSize, setFontSize] = useState(String(defaultBatesFormat.fontSize));
    const [files, setFiles] = useState<readonly string[]>(initialPaths);
    const [pageCounts, setPageCounts] = useState<PageCounts>(new Map());
    const [outcomes, setOutcomes] = useState<ReadonlyMap<string, JobOutcome>>(new Map());
    const [copied, setCopied] = useState(false);
    // The whole ordered set, read when a run starts: a retry runs one file but must number it within the full set.
    const filesRef = useRef<readonly string[]>(initialPaths);
    const requested = useRef(new Set<string>());

    const format: BatesFormat = {
        prefix,
        suffix,
        start: start.trim() === "" ? Number.NaN : Number(start),
        digits: digits.trim() === "" ? Number.NaN : Number(digits),
        position,
        fontFamily,
        fontSize: fontSize.trim() === "" ? Number.NaN : Number(fontSize),
    };
    const formatIssue = batesFormatIssue(format);

    const filesChanged = useCallback((paths: readonly string[]) => {
        filesRef.current = paths;
        setFiles(paths);
        setOutcomes(new Map());
        setCopied(false);
        onFilesChange?.(paths);
    }, [onFilesChange]);

    useEffect(() => {
        for (const path of files) {
            if (requested.current.has(path)) continue;
            requested.current.add(path);
            invoke<{ pages: unknown[] }>("inspect_pdf_scene", { request: { path } })
                .then((document) => document.pages.length)
                .catch(() => null)
                .then((count) => setPageCounts((counts) => new Map(counts).set(path, count)));
        }
    }, [files]);

    const counts = files.map((path) => pageCounts.get(path));
    const ranges = formatIssue ? files.map(() => null) : assignBatesRanges(format.start, counts.map((count) => count ?? null));
    const plannedRange = (path: string) => {
        const index = files.indexOf(path);
        const range = ranges[index];
        return range ? batesRangeLabel(format, range) : null;
    };

    const numbersFor = (path: string) => {
        const outcome = outcomes.get(path);
        const reported = outcome && reportedNumbers(outcome);
        if (reported?.unused) {
            return (
                <span className="bates-numbers" data-state="unused">
                    <s>{reported.range}</s>
                    <span className="bates-numbers-note">Unused gap</span>
                </span>
            );
        }
        if (reported) return <span className="bates-numbers" data-state="stamped">{reported.range}</span>;
        if (outcome) return <span className="bates-numbers" data-state="none">None</span>;
        const count = pageCounts.get(path);
        if (count === undefined) return <span className="bates-numbers" data-state="pending">Counting pages…</span>;
        if (count === null) return <span className="bates-numbers" data-state="none">Page count unknown</span>;
        return <span className="bates-numbers" data-state="planned">{plannedRange(path) ?? `${count} ${count === 1 ? "page" : "pages"}`}</span>;
    };

    const finished = files.filter((path) => outcomes.has(path)).map((path) => ({ path, outcome: outcomes.get(path)! }));
    const numbered = finished.filter(({ outcome }) => outcome.state === "succeeded").length;
    const failed = finished.length - numbered;
    const gaps = finished.map(({ outcome }) => reportedNumbers(outcome)).filter((reported) => reported?.unused).map((reported) => reported!.range);
    const unnumbered = finished.filter(({ outcome }) => !reportedNumbers(outcome)).map(({ path }) => fileName(path));

    const copyNumberList = () => {
        const lines = finished.map(({ path, outcome }) => {
            const reported = reportedNumbers(outcome);
            if (!reported) return `${fileName(path)}\tNone`;
            return `${fileName(path)}\t${reported.range}${reported.unused ? "\tUnused gap" : ""}`;
        });
        void navigator.clipboard?.writeText(lines.join("\n")).then(() => setCopied(true), () => setCopied(false));
    };

    return (
        <ToolScaffold
            key={initialPaths.join("\0") || "no-initial-pdf"}
            variant="workspace"
            sessionKey="pdf-bates"
            utility={utility}
            initialPaths={initialPaths}
            onFilesChange={filesChanged}
            renderFileExtra={(path) => numbersFor(path)}
            onRun={async (paths, outputLocation, { runId }) => {
                const result = await invoke<ToolResult>("bates_number_pdfs", {
                    runId,
                    request: { paths: filesRef.current, run: paths, ...format, outputLocation },
                });
                if (Array.isArray(result)) {
                    setOutcomes((current) => {
                        const next = new Map(current);
                        for (const outcome of result) next.set(outcome.inputPath, outcome);
                        return next;
                    });
                }
                return result;
            }}
        >
            {({ files: runFiles, run, loading, destination }) => (
                <Card className="workspace-control-panel py-0">
                    <CardContent className="workspace-control-panel-content grid gap-4 p-4">
                        <h2 className="workspace-panel-label">Numbering</h2>
                        {loading && <p className="workspace-note">Locked while numbering runs.</p>}
                        <div className="workspace-field-grid">
                            <div className="workspace-field">
                                <Label htmlFor="bates-prefix">Prefix</Label>
                                <Input id="bates-prefix" value={prefix} placeholder="None" onChange={(event) => setPrefix(event.target.value)} />
                            </div>
                            <div className="workspace-field">
                                <Label htmlFor="bates-suffix">Suffix</Label>
                                <Input id="bates-suffix" value={suffix} placeholder="None" onChange={(event) => setSuffix(event.target.value)} />
                            </div>
                            <div className="workspace-field">
                                <Label htmlFor="bates-start">Start at</Label>
                                <Input id="bates-start" inputMode="numeric" value={start} onChange={(event) => setStart(event.target.value)} />
                            </div>
                            <div className="workspace-field">
                                <Label htmlFor="bates-digits">Digits</Label>
                                <Input id="bates-digits" inputMode="numeric" value={digits} onChange={(event) => setDigits(event.target.value)} />
                            </div>
                            <div className="workspace-field workspace-field-wide">
                                <Label htmlFor="bates-position">Position</Label>
                                <NativeSelect id="bates-position" value={position} onChange={(event) => setPosition(event.target.value as BatesPosition)}>
                                    {batesPositions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                                </NativeSelect>
                            </div>
                            <div className="workspace-field">
                                <Label htmlFor="bates-font">Font</Label>
                                <NativeSelect id="bates-font" value={fontFamily} onChange={(event) => setFontFamily(event.target.value)}>
                                    {textFonts.map((family) => <option key={family} value={family}>{family}</option>)}
                                </NativeSelect>
                            </div>
                            <div className="workspace-field">
                                <Label htmlFor="bates-size">Size</Label>
                                <Input id="bates-size" inputMode="decimal" value={fontSize} onChange={(event) => setFontSize(event.target.value)} />
                            </div>
                        </div>
                        <div className="workspace-field">
                            <span className="workspace-panel-label" id="bates-example-label">Example</span>
                            <output className="bates-example" aria-labelledby="bates-example-label" data-testid="bates-example">
                                {formatIssue ? "—" : batesIdentifier(format, format.start)}
                            </output>
                        </div>
                        {formatIssue && <p className="workspace-note" role="alert">{formatIssue}</p>}
                        <p className="workspace-note">Each file is saved with -bates added to its name. Originals are never changed.</p>
                        {destination}
                        <Button
                            variant="default"
                            disabled={loading || runFiles.length === 0 || formatIssue !== null}
                            onClick={run}
                            className="workspace-primary-action"
                            {...shortcutTarget("run")}
                        >
                            {runFiles.length === 0 ? "Number files" : `Number ${runFiles.length} ${runFiles.length === 1 ? "file" : "files"}`}
                            <ShortcutHint id="run" />
                        </Button>
                        {!loading && finished.length > 0 && (
                            <div className="bates-summary" role="status">
                                <p className="workspace-note">
                                    <strong>
                                        {numbered} {numbered === 1 ? "file" : "files"} numbered{failed > 0 ? `, ${failed} failed` : ""}.
                                        {gaps.length > 0 && ` ${gaps.join(", ")} ${gaps.length === 1 && !gaps[0].includes(" to ") ? "is" : "are"} unused.`}
                                    </strong>
                                    {gaps.length > 0 && " Those numbers were set aside for files that failed, so later files keep their numbers and the set has a gap."}
                                    {unnumbered.length > 0 && ` No numbers were set aside for ${unnumbered.join(", ")}; later files continued from the next number.`}
                                </p>
                                <Button variant="outline" size="sm" onClick={copyNumberList}>{copied ? "Copied" : "Copy number list"}</Button>
                            </div>
                        )}
                    </CardContent>
                </Card>
            )}
        </ToolScaffold>
    );
};
