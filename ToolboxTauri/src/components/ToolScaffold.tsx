import { createContext, useContext, useState, useEffect, useMemo, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select';
import type { ToolDefinition, JobOutcome, OutputLocation, Progress } from '../contracts';
import { TOOL_DROP_EVENT, type ShellEvent } from '../hooks/useShellBridge';
import { useOutputLocation } from '../outputLocation';
import { failedOutcome, parseToolResult } from '../jobOutcome';
import { UtilityRegistry, toolsForWorkspace, workspaceForTool } from '../registry';
import { shortenHomePath, useHomeDir } from '../homePath';
import { CancelRun } from './CancelRun';
import { ShortcutHint } from './ShortcutHint';
import { isWindows, matchShortcut, shortcutTarget } from '../shortcuts';
import { ResultList, followPointer } from './ResultList';
import { rowView, type OutputGrouping } from './resultRow';
import { BATCH_PROGRESS_EVENT, foldRunEvent, parseRunEvent, startRun, type LiveRun } from './liveRun';
import { BlockedDestinationDialog, RunDestination, exampleOutputName, sameLocation, type BlockedDestination, type FolderCheck } from './RunDestination';
import { TablerIcon } from './TablerIcon';
import { chooseCreatedDocumentSavePath, chooseManyToOneSavePath } from './saveStep';
import { PasteHtmlDialog } from './PasteHtmlDialog';
import { addPastedHtml, choosePastedOutput, isPastedHtml, pastedHtml, pastedRowDetail } from './pastedHtml';
import { saveStepAppliesTo, saveStepFor, type SaveDialog } from '../registry/saveSteps';
import { ArrowDown, ArrowUp, FileIcon, FolderIcon, GripVertical, XIcon } from 'lucide-react';
import {
    classifyIntake,
    entrySortOptions,
    entryStatuses,
    fileName,
    fileSizeLabel,
    modifiedLabel,
    pageCountLabel,
    intakePolicyFor,
    moveEntry,
    parentFolder,
    parseEntryFacts,
    parseInspection,
    rangeKeys,
    sortEntries,
    sortNeedsFacts,
    type EntryFacts,
    type EntrySort,
    type EntrySortOption,
    type InspectedPath,
    type IntakeCopy,
    type IntakeEntry,
    type IntakeRedirect,
    type NotAdded,
} from './intake';


const repeatLabel = (occurrence: number) => occurrence === 2 ? 'Added twice' : `Added ${occurrence} times`;

export type WorkspaceSourceAction = () => void | Promise<void>;

/** Asks before work that is not exported yet is dropped; calls proceed to go ahead or cancel to keep it. */
export type DiscardGuard = (proceed: () => void, cancel?: () => void) => void;

// The view rail lives outside the scaffold, so a run reports here to lock it like the options fieldset.
export const RunStateContext = createContext<((running: boolean) => void) | null>(null);

/** `keys` names each run path's list entry, in the same order, so a tool can tell repeated files apart. */
export type RunContext = { outputPath: string | null; runId: string; keys: string[] };

/** What a report can do after its run: retry one file, and save to this run's destination. */
export interface ReportRun {
    withPassword?: (inputPath: string, password: string) => Promise<void>;
    destination: { location: OutputLocation; change: () => void };
    /** Checks the destination folder first and offers another one when it can't be used. */
    saveTo: (save: (location: OutputLocation) => Promise<void>) => Promise<void>;
}

export type RunHandler = (files: string[], outputLocation: OutputLocation, run: RunContext) => Promise<JobOutcome[]>;

/** Save As for a tool that writes one file per input: the person names the file in the shared, never-replace save step. */
export type SaveAs = { dialog: SaveDialog; suffix: string; summary: string };

type PerFileRun = { handler: RunHandler; outputLocation: OutputLocation };

export type RunDestinationControl = {
    location: OutputLocation;
    defaultLocation: OutputLocation;
    overridden: boolean;
    /** Opens the folder dialog and uses the chosen folder for this run only. Settings stay unchanged. */
    change: () => void;
    reset: () => void;
};

interface ToolScaffoldProps {
    utility: ToolDefinition;
    onRun: RunHandler;
    onRunCombined?: RunHandler;
    variant?: 'standard' | 'workspace';
    sessionKey?: string;
    onWorkspaceSourceAction?: (action: WorkspaceSourceAction | null) => void;
    showFileOrdering?: boolean;
    renderFileExtra?: (path: string, selected: boolean, locked: boolean) => React.ReactNode;
    /** A warning shown in place of a runnable file's folder, like a signature that saving removes. */
    fileWarning?: (path: string) => string | null;
    retryWithPassword?: (inputPath: string, password: string) => RunHandler;
    intakeCopy?: IntakeCopy;
    intakeRedirect?: (path: string) => IntakeRedirect | null;
    /** What the workspace source bar says before any file is open, such as the formats it takes. */
    emptyHint?: string;
    initialPaths?: readonly string[];
    fileActivation?: Extract<ShellEvent, { kind: 'files' }>;
    onActivationAccepted?: (activationId: string) => void;
    onFilesChange?: (paths: readonly string[]) => void;
    confirmDiscard?: DiscardGuard;
    /** Overrides the example output name when the view's options decide the output's suffix or type. */
    exampleName?: (inputPath: string) => string | readonly string[];
    /** Offers Paste HTML: pasted markup joins the list and asks where to save when the run starts. */
    pasteHtml?: boolean;
    exampleHint?: string;
    /** Sorts an ordered list offers. The list keeps the order a sort leaves, so later drags start from it. */
    sortOptions?: readonly EntrySortOption[];
    /** Shows a succeeded file's outputs as one row that expands into its parts. */
    outputGroup?: OutputGrouping;
    /** A line after the finished headline, such as a total across files. */
    resultsHint?: (results: JobOutcome[]) => string | null;
    /** Facts about each listed file. An ordered list given facts shows page counts, sizes, dates and a sort control. */
    entryFacts?: (path: string) => EntryFacts | undefined;
    /** Replaces the list's default hint, for example when the view needs more files. */
    listHint?: string;
    /**
     * Marks a read-only tool: the run skips the output-folder check and the finished
     * results show through this instead of the result list.
     */
    renderReport?: (results: JobOutcome[], run: ReportRun) => React.ReactNode;
    /** Replaces the destination's label and example for a tool whose saves are not its run. */
    destinationCopy?: (files: string[]) => { label: string; note: string };
    /** Says in the per-run destination hint that the saved setting stays as it is. */
    destinationMentionsSettingKept?: boolean;
    children: (props: {
        files: string[];
        failingFiles: string[];
        outputLocation: OutputLocation;
        run: () => Promise<void>;
        runCombined: () => Promise<void>;
        runSavingAs: (saveAs: SaveAs) => Promise<void>;
        loading: boolean;
        progress: Progress;
        selectedFileIndex: number;
        selectFile: (index: number) => void;
        /** Each file's list entry key, parallel to `files`; a file added twice has two keys. */
        fileKeys: string[];
        /** Moves the file at `index` of `files` to `toIndex`, through the same ordering as the file list. */
        moveFile: (index: number, toIndex: number) => void;
        /** Removes the file at `index` of `files` from the list, never from disk. */
        removeFile: (index: number) => void;
        destination: React.ReactNode;
        /** The pieces of `destination`, for a view that places its own destination control, such as next to Export. */
        runDestination: RunDestinationControl;
        live: LiveRun | null;
        runLabel: string | null;
        failedFiles: string[];
        retryFailed: () => Promise<void>;
        retrying: boolean;
        /** Opens the file dialog, as the workspace's own open button does. */
        browse: () => Promise<void>;
        /** Replaces the open files, for views that decide which dropped file to keep. */
        replaceFiles: (paths: readonly string[]) => void;
    }) => React.ReactNode;
}

export const ToolScaffold = ({ utility, onRun, onRunCombined, variant = 'standard', sessionKey, onWorkspaceSourceAction, showFileOrdering = true, renderFileExtra, fileWarning, retryWithPassword, intakeCopy, intakeRedirect, emptyHint = 'Drop files here or browse from this device.', initialPaths = [], fileActivation, onActivationAccepted, onFilesChange, confirmDiscard, exampleName, exampleHint, pasteHtml = false, sortOptions, outputGroup, resultsHint, entryFacts, listHint: listHintOverride, renderReport, destinationCopy, destinationMentionsSettingKept = false, children }: ToolScaffoldProps) => {
    const nextEntryKey = useRef(0);
    const newEntries = (items: readonly (string | InspectedPath)[]): IntakeEntry[] => items.map((item) => typeof item === 'string'
        ? { key: `entry-${++nextEntryKey.current}`, path: item }
        : { key: `entry-${++nextEntryKey.current}`, path: item.path, animated: item.animated });
    const [entries, setEntries] = useState<IntakeEntry[]>(() => newEntries(initialPaths));
    const [selection, setSelection] = useState<ReadonlySet<string>>(() => new Set());
    const [anchor, setAnchor] = useState<string | null>(null);
    const [notAdded, setNotAdded] = useState<NotAdded[]>([]);
    const [moveStatus, setMoveStatus] = useState('');
    const [dragKey, setDragKey] = useState<string | null>(null);
    const [dropTargetKey, setDropTargetKey] = useState<string | null>(null);
    const [sort, setSort] = useState<EntrySort>('added');
    const rowButtons = useRef(new Map<string, HTMLButtonElement>());
    const home = useHomeDir();
    const [results, setResults] = useState<JobOutcome[]>([]);
    const [combinedInputs, setCombinedInputs] = useState<string[] | null>(null);
    const [loading, setLoading] = useState(false);
    const [live, setLive] = useState<LiveRun | null>(null);
    const [perFileRun, setPerFileRun] = useState<PerFileRun | null>(null);
    const [retrying, setRetrying] = useState(false);
    const [pasting, setPasting] = useState(false);
    const defaultLocation = useOutputLocation();
    const [runOverride, setRunOverride] = useState<OutputLocation | null>(null);
    const [blocked, setBlocked] = useState<(BlockedDestination & { retry: (location: OutputLocation) => void }) | null>(null);
    const outputLocation = runOverride ?? defaultLocation;
    const runGeneration = useRef(0);
    const browseRef = useRef<WorkspaceSourceAction>(() => undefined);
    const inputPolicy = utility.capability;
    const workspace = variant === 'workspace' ? workspaceForTool(utility.id) : undefined;
    const intakeTools = useMemo(() => workspace ? toolsForWorkspace(workspace) : [utility], [workspace, utility]);
    const ordered = inputPolicy.inputCardinality === 'ordered' && showFileOrdering;
    const describesEntries = ordered && entryFacts !== undefined;
    const copiesByPath = useMemo(() => {
        const counts = new Map<string, number>();
        for (const entry of entries) counts.set(entry.path, (counts.get(entry.path) ?? 0) + 1);
        return counts;
    }, [entries]);
    const statuses = useMemo(() => entryStatuses(entries, utility, intakeCopy), [entries, utility, intakeCopy]);
    const runnable = useMemo(() => statuses.filter((status) => status.flag?.kind !== 'wontRun'), [statuses]);
    const files = useMemo(() => runnable.map((status) => status.entry.path), [runnable]);
    const fileKeys = useMemo(() => runnable.map((status) => status.entry.key), [runnable]);
    const failingFiles = useMemo(() => runnable.filter((status) => status.flag?.kind === 'willFail').map((status) => status.entry.path), [runnable]);
    const flagged = statuses.filter((status) => status.flag !== null);
    const selectedFileIndex = Math.max(runnable.findIndex((status) => status.entry.key === anchor), 0);
    const saveStep = saveStepFor(utility);
    const savesOneFile = saveStepAppliesTo(saveStep, files.length);
    const progress: Progress = {
        completed: loading ? 0 : Math.min(results.length, files.length),
        total: files.length,
    };

    const resolvedSessionKey = sessionKey ?? utility.id;

    useEffect(() => {
        runGeneration.current += 1;
        setResults([]);
        setLoading(false);
        setLive(null);
        setSelection(new Set());
        setAnchor(null);
    }, [resolvedSessionKey]);

    useEffect(() => {
        runGeneration.current += 1;
        setResults([]);
        setLoading(false);
        setLive(null);
        setRunOverride(null);
        setBlocked(null);
    }, [utility.id]);

    const overrideWith = (location: OutputLocation) =>
        setRunOverride(sameLocation(location, defaultLocation) ? null : location);

    useEffect(() => {
        runGeneration.current += 1;
        setResults([]);
    }, [files]);

    const runStreamed = async (handler: RunHandler, paths: string[], keys: string[], location: OutputLocation, outputPath: string | null, generation: number) => {
        const runId = crypto.randomUUID();
        setLive(startRun(runId, paths));
        let unlisten: UnlistenFn | null = null;
        try {
            unlisten = await listen(BATCH_PROGRESS_EVENT, ({ payload }) => {
                const event = parseRunEvent(payload);
                if (event && generation === runGeneration.current) setLive((run) => run && foldRunEvent(run, event));
            });
        } catch {
            // Without progress events the run still completes and shows its final results.
        }
        try {
            return parseToolResult(await handler(paths, location, { outputPath, runId, keys }));
        } finally {
            unlisten?.();
        }
    };

    const runWith = async (handler: RunHandler, location: OutputLocation = outputLocation, saveAs: SaveAs | null = null) => {
        const generation = ++runGeneration.current;
        setLoading(true);
        setResults([]);
        setCombinedInputs(null);
        setPerFileRun(null);
        try {
            // Save As names its own folder in the save step, and a report saves nothing, so the default destination isn't checked.
            if (!renderReport && location !== 'alongsideInput' && !saveAs) {
                const check = await invoke<FolderCheck>('check_output_folder', { folder: location.customFolder });
                if (check !== 'ready') {
                    if (generation === runGeneration.current) setBlocked({ folder: location.customFolder, check, retry: (next) => void runWith(handler, next) });
                    return;
                }
            }
            // A retry reruns without the save step, so a Save As run offers none rather than writing a different name.
            if (!inputPolicy.manyToOne && !saveAs) setPerFileRun({ handler, outputLocation: location });
            const cardinalityError = inputPolicy.inputCardinality === 'single' && files.length > 1
                ? `This action accepts one input file, but ${files.length} were selected.`
                : null;
            const availabilityError = inputPolicy.nativeAvailability === 'unavailable'
                ? `${utility.title} is unavailable in this build.`
                : null;
            const invalidPaths = new Map<string, JobOutcome>();
            const validKeys = fileKeys.filter((_, index) => {
                const path = files[index];
                if (cardinalityError) {
                    invalidPaths.set(path, failedOutcome(path, 'invalidInput', cardinalityError));
                    return false;
                }
                if (availabilityError) {
                    invalidPaths.set(path, failedOutcome(path, 'unavailable', availabilityError));
                    return false;
                }
                return true;
            });
            const validPaths = validKeys.map((key) => files[fileKeys.indexOf(key)]);
            const orderedSelectionRejected = inputPolicy.inputCardinality === 'ordered' && invalidPaths.size > 0;
            const runs = !orderedSelectionRejected && validPaths.length > 0;
            let outputPath: string | null = null;
            const chosenSave = saveAs ?? (saveStepAppliesTo(saveStep, validPaths.length)
                ? { dialog: saveStep, suffix: inputPolicy.outputSuffix, summary: saveStep.summary(validPaths.length) }
                : null);
            if (runs && chosenSave) {
                setLoading(false);
                outputPath = await chooseManyToOneSavePath(chosenSave.dialog, validPaths, location, chosenSave.suffix, chosenSave.summary);
                if (!outputPath || generation !== runGeneration.current) return;
                setLoading(true);
            }
            const pastedPaths = runs ? validPaths.filter(isPastedHtml) : [];
            if (pastedPaths.length > 0) {
                setLoading(false);
                const others = validPaths.length - pastedPaths.length;
                const elsewhere = others === 0 ? '' : `The other ${others === 1 ? 'file saves' : `${others} files save`} ${location === 'alongsideInput' ? 'next to each original' : 'in the folder chosen for this run'}. `;
                for (const path of pastedPaths) {
                    const item = pastedHtml(path);
                    const chosen = item && await chooseCreatedDocumentSavePath(
                        { title: 'Save the pasted HTML as a PDF', extension: 'pdf' },
                        `${item.name}.pdf`,
                        `Opens in the folder you last saved to, or Documents. ${elsewhere}Cancel goes back to the list without converting.`,
                    );
                    if (!chosen || generation !== runGeneration.current) return;
                    choosePastedOutput(path, chosen);
                }
                setLoading(true);
            }
            const processed = runs ? await runStreamed(handler, validPaths, validKeys, location, outputPath, generation) : [];
            const processedByPath = new Map(processed.map((result) => [result.inputPath, result]));
            const nextResults = orderedSelectionRejected
                ? files.map((path) => invalidPaths.get(path) ?? failedOutcome(path, 'invalidInput', 'Every selected input must use a supported format for this ordered action.'))
                : invalidPaths.size === 0
                    ? processed
                : files.map((path) => invalidPaths.get(path) ?? processedByPath.get(path)).filter((result): result is JobOutcome => result !== undefined);
            if (generation === runGeneration.current) {
                setResults(nextResults);
                setCombinedInputs(outputPath === null ? null : validPaths);
            }
        } catch (error) {
            if (generation === runGeneration.current) {
                setResults([failedOutcome('Tool error', 'processing', String(error))]);
            }
        } finally {
            if (generation === runGeneration.current) {
                setLoading(false);
                setLive(null);
            }
        }
    };
    const run = () => runWith(onRun);
    const runCombined = () => runWith(onRunCombined ?? onRun);
    const runSavingAs = (saveAs: SaveAs) => runWith(onRun, outputLocation, saveAs);

    const chooseRunFolder = async () => {
        const picked = await open({ directory: true, multiple: false });
        return typeof picked === 'string' ? picked : null;
    };

    const changeForRun = async () => {
        try {
            const folder = await chooseRunFolder();
            if (folder) overrideWith({ customFolder: folder });
        } catch (e) {
            setResults([failedOutcome('Dialog error', 'processing', String(e))]);
        }
    };

    const saveBlockedAlongside = () => {
        if (!blocked) return;
        setBlocked(null);
        overrideWith('alongsideInput');
        blocked.retry('alongsideInput');
    };

    const chooseBlockedFolder = async () => {
        if (!blocked) return;
        let folder: string | null;
        try {
            folder = await chooseRunFolder();
        } catch (e) {
            setBlocked(null);
            setResults([failedOutcome('Dialog error', 'processing', String(e))]);
            return;
        }
        if (!folder) return;
        const location = { customFolder: folder };
        setBlocked(null);
        overrideWith(location);
        blocked.retry(location);
    };

    const saveTo = async (save: (location: OutputLocation) => Promise<void>, location: OutputLocation = outputLocation) => {
        if (location !== 'alongsideInput') {
            const check = await invoke<FolderCheck>('check_output_folder', { folder: location.customFolder });
            if (check !== 'ready') {
                setBlocked({ folder: location.customFolder, check, retry: (next) => void saveTo(save, next) });
                return;
            }
        }
        await save(location);
    };

    // A rerun is per file, where a file appears once, so its path finds its entry.
    const rerun = async (paths: string[], handler = perFileRun?.handler) => {
        if (!perFileRun || !handler || paths.length === 0) return;
        const generation = runGeneration.current;
        let rerunResults: JobOutcome[];
        try {
            rerunResults = parseToolResult(await handler(paths, perFileRun.outputLocation, { outputPath: null, runId: crypto.randomUUID(), keys: paths.map((path) => fileKeys[files.indexOf(path)] ?? path) }));
        } catch (error) {
            rerunResults = paths.map((path) => failedOutcome(path, 'processing', String(error)));
        }
        if (generation !== runGeneration.current) return;
        const replacements = new Map(rerunResults.filter((result) => paths.includes(result.inputPath)).map((result) => [result.inputPath, result]));
        setResults((current) => current.map((result) => replacements.get(result.inputPath) ?? result));
    };

    const failedFiles = perFileRun
        ? results.filter((result) => {
            const view = rowView(result, utility.resultVerb);
            return view.canRetry || (retryWithPassword !== undefined && view.canRetryWithPassword);
        }).map((result) => result.inputPath)
        : [];

    const retryFailed = async () => {
        setRetrying(true);
        try {
            await rerun(failedFiles, onRun);
        } finally {
            setRetrying(false);
        }
    };

    const discard: DiscardGuard = (proceed, cancel) => confirmDiscard ? confirmDiscard(proceed, cancel) : proceed();

    const selectOnly = (key: string | null) => {
        setSelection(new Set(key ? [key] : []));
        setAnchor(key);
    };

    const addPasted = (name: string, markup: string) => {
        setPasting(false);
        const [entry] = newEntries([addPastedHtml(name, markup)]);
        setNotAdded([]);
        setEntries((current) => [...current, entry]);
        selectOnly(entry.key);
    };

    const pasteButton = pasteHtml && (
        <Button variant="outline" size="sm" disabled={loading} onClick={() => setPasting(true)}>
            Paste HTML…
        </Button>
    );
    const hasPasted = entries.some((entry) => isPastedHtml(entry.path));

    const intake = (incoming: InspectedPath[], replaceSingle = false) => {
        const policy = { ...intakePolicyFor(workspace?.title ?? utility.title, intakeTools, utility), notAcceptedMessage: intakeCopy?.notAccepted, keepsUnsupported: intakeCopy?.keepsUnsupported, redirect: intakeRedirect };
        if (inputPolicy.inputCardinality === 'single' && replaceSingle) {
            // A picked file still goes through intake rules, so a file the tool can't open never replaces the open one.
            const classified = classifyIntake([], incoming.slice(0, 1), policy);
            setNotAdded(classified.notAdded);
            const next = classified.added[0];
            if (!next || (entries.length === 1 && entries[0].path === next.path)) return;
            discard(() => {
                const [entry] = newEntries([next]);
                setEntries([entry]);
                selectOnly(entry.key);
            });
            return;
        }
        const classified = classifyIntake(entries.map((entry) => entry.path), incoming, policy);
        setNotAdded(classified.notAdded);
        if (classified.added.length) setEntries((current) => [...current, ...newEntries(classified.added)]);
    };

    const inspect = async (paths: string[]) => {
        try {
            return parseInspection(await invoke('inspect_intake_paths', { paths }), paths);
        } catch {
            return parseInspection(null, paths);
        }
    };

    const dropPaths = async (paths: string[]) => intake(await inspect(paths));

    const dropRef = useRef<(paths: string[]) => void>(() => undefined);
    dropRef.current = (paths) => {
        if (!loading) void dropPaths(paths);
    };

    // The native shell owns window drops and forwards them here once a tool is open.
    useEffect(() => {
        const handleDroppedFiles = (event: Event) => {
            const paths = (event as CustomEvent<unknown>).detail;
            if (Array.isArray(paths) && paths.every((path) => typeof path === 'string')) dropRef.current(paths);
        };
        window.addEventListener(TOOL_DROP_EVENT, handleDroppedFiles);
        return () => window.removeEventListener(TOOL_DROP_EVENT, handleDroppedFiles);
    }, []);

    // Files opened from Finder, Explorer or the dock replace the current files once.
    const [acceptedActivationId, setAcceptedActivationId] = useState<string | null>(null);
    useEffect(() => {
        if (!fileActivation || loading) return;
        const { activationId, paths } = fileActivation;
        discard(() => {
            const activated = newEntries(paths);
            setEntries(activated);
            setResults([]);
            setNotAdded([]);
            selectOnly(activated[0]?.key ?? null);
            setAcceptedActivationId(activationId);
        }, () => {
            onFilesChange?.(entryPaths);
            setAcceptedActivationId(activationId);
        });
    }, [fileActivation?.activationId]);

    useEffect(() => {
        if (!fileActivation || acceptedActivationId !== fileActivation.activationId) return;
        void invoke('acknowledge_activation', { activationId: fileActivation.activationId }).then(() => {
            onActivationAccepted?.(fileActivation.activationId);
        });
    }, [acceptedActivationId, fileActivation, onActivationAccepted]);

    const entryPaths = useMemo(() => entries.map((entry) => entry.path), [entries]);
    useEffect(() => {
        onFilesChange?.(entryPaths);
    }, [entryPaths, onFilesChange]);

    const browse = async () => {
        if (loading) return;
        try {
            const picked = await open({
                multiple: inputPolicy.inputCardinality !== 'single',
                filters: [{ name: 'Supported files', extensions: inputPolicy.acceptedExtensions.map((extension) => extension.replace(/^\./, '')) }],
            });
            const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
            if (paths.length) intake(await inspect(paths), true);
        } catch (e) {
            setResults([failedOutcome('Dialog error', 'processing', String(e))]);
        }
    };

    browseRef.current = browse;

    const replaceFiles = (paths: readonly string[]) => {
        if (loading) return;
        const next = newEntries(paths);
        setEntries(next);
        setResults([]);
        selectOnly(next[0]?.key ?? null);
    };
    const reportRunState = useContext(RunStateContext);
    useEffect(() => {
        if (!reportRunState) return;
        reportRunState(loading);
        return () => reportRunState(false);
    }, [loading, reportRunState]);
    useEffect(() => {
        if (variant !== 'workspace' || !onWorkspaceSourceAction) return;
        onWorkspaceSourceAction(entries.length === 0 ? () => browseRef.current() : null);
        return () => onWorkspaceSourceAction(null);
    }, [entries.length, onWorkspaceSourceAction, variant]);

    const removeEntries = (keys: ReadonlySet<string>) => {
        if (loading || keys.size === 0) return;
        discard(() => {
            setEntries((current) => current.filter((entry) => !keys.has(entry.key)));
            selectOnly(null);
            setMoveStatus('');
        });
    };

    const clearFiles = () => {
        if (loading) return;
        discard(() => {
            setEntries([]);
            setResults([]);
            setNotAdded([]);
            setMoveStatus('');
            setSort('added');
            selectOnly(null);
        });
    };

    const addedOrder = (entry: IntakeEntry) => Number(entry.key.slice('entry-'.length));

    const sortBy = async (next: EntrySort) => {
        if (loading) return;
        setSort(next);
        setMoveStatus('');
        let facts = (path: string) => entryFacts?.(path);
        if (entryFacts === undefined && sortNeedsFacts(next)) {
            // A list without facts of its own reads the dates once, when a sort needs them.
            let described = new Map<string, EntryFacts>();
            try {
                described = parseEntryFacts(await invoke('describe_intake_files', { paths: [...new Set(entries.map((entry) => entry.path))] }));
            } catch {
                // Files whose dates can't be read sort last, in their current order.
            }
            facts = (path) => described.get(path);
        }
        setEntries((current) => sortEntries(current, next, facts, addedOrder));
    };

    const selectFile = (index: number) => selectOnly(runnable[index]?.entry.key ?? null);

    const clickRow = (key: string, event: React.MouseEvent) => {
        if (event.shiftKey && anchor) {
            setSelection(new Set(rangeKeys(entries, anchor, key)));
            return;
        }
        if (event.metaKey || event.ctrlKey) {
            setSelection((current) => {
                const next = new Set(current);
                if (next.has(key)) next.delete(key);
                else next.add(key);
                return next;
            });
            setAnchor(key);
            return;
        }
        selectOnly(key);
    };

    const focusRow = (key: string) => requestAnimationFrame(() => rowButtons.current.get(key)?.focus());

    const moveTo = (key: string, toIndex: number, focus = true) => {
        if (loading) return;
        const next = moveEntry(entries, key, toIndex);
        if (next === entries) return;
        setEntries(next);
        selectOnly(key);
        const moved = next[toIndex];
        setMoveStatus(`Moved ${fileName(moved.path)} to position ${toIndex + 1}.`);
        if (focus) focusRow(key);
    };

    // A view's own arrangement of the files, such as the GIF frame grid, keeps focus where it is.
    const moveFile = (index: number, toIndex: number) => {
        const key = fileKeys[index];
        const target = fileKeys[toIndex];
        if (!ordered || key === undefined || target === undefined) return;
        moveTo(key, entries.findIndex((entry) => entry.key === target), false);
    };
    const removeFile = (index: number) => {
        const key = fileKeys[index];
        if (key !== undefined) removeEntries(new Set([key]));
    };

    const listKeyDown = (event: React.KeyboardEvent<HTMLUListElement>) => {
        if (loading) return;
        const delta = event.key === 'ArrowUp' ? -1 : event.key === 'ArrowDown' ? 1 : 0;
        const index = entries.findIndex((entry) => entry.key === anchor);
        if (delta && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            if (ordered && index >= 0) moveTo(entries[index].key, index + delta);
            return;
        }
        const shortcut = matchShortcut(event.nativeEvent, 'file-list');
        if (shortcut === 'remove') {
            event.preventDefault();
            removeEntries(selection);
            return;
        }
        if (shortcut === 'selectAll') {
            event.preventDefault();
            setSelection(new Set(entries.map((entry) => entry.key)));
            return;
        }
        if (shortcut !== 'move' || index < 0) return;
        event.preventDefault();
        const target = entries[index + delta];
        if (!target) return;
        if (event.shiftKey) setSelection((current) => new Set([...current, target.key]));
        else setSelection(new Set([target.key]));
        setAnchor(target.key);
        focusRow(target.key);
    };

    const dropTargetAt = (x: number, y: number) =>
        document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-intake-key]')?.dataset.intakeKey ?? null;

    const endDrag = (event: React.PointerEvent) => {
        const key = dragKey;
        setDragKey(null);
        setDropTargetKey(null);
        if (!key) return;
        const target = dropTargetAt(event.clientX, event.clientY);
        const toIndex = entries.findIndex((entry) => entry.key === target);
        if (toIndex >= 0) moveTo(key, toIndex);
    };

    const modifier = isWindows() ? 'Ctrl' : '⌘';
    const listHint = listHintOverride ?? (describesEntries
        ? `Drag a row by its handle, or select it and press ${modifier}+${isWindows() ? 'Up' : '↑'} or ${modifier}+${isWindows() ? 'Down' : '↓'}. The top file comes first.`
        : ordered
        ? `Files run top to bottom. Drag a row, or select it and press ${modifier}+Up or ${modifier}+Down.`
        : `Shift-click or ${isWindows() ? 'Ctrl-click' : '⌘-click'} to select. Delete removes files from the list, not from disk.`);
    const pageCounts = entries.map((entry) => entryFacts?.(entry.path)?.pageCount ?? null);
    const totalPages = describesEntries && pageCounts.length > 0 && pageCounts.every((count) => count !== null)
        ? pageCounts.reduce<number>((sum, count) => sum + (count ?? 0), 0)
        : null;

    const selectionNote = [
        selection.size > 0 ? `${selection.size} selected` : null,
        intakeCopy?.countHint(flagged.length, entries.length) ?? null,
    ].filter(Boolean).join('. ');

    const fileSelection = (
        <div className="file-selection" data-locked={loading ? 'true' : undefined}>
            <div className="file-selection-header">
                <span className="file-selection-count">{entries.length} {entries.length === 1 ? 'file' : 'files'}</span>
                {totalPages !== null && <span className="file-selection-selected">{totalPages === 1 ? '1 page in all' : `${totalPages} pages in all`}</span>}
                {selectionNote && <span className="file-selection-selected">{selectionNote}</span>}
                {ordered && (sortOptions !== undefined || describesEntries) && (
                    <label className="file-selection-sort">
                        <span>{sortOptions ? 'Sort by' : 'Sort'}</span>
                        <NativeSelect size="sm" value={sort} disabled={loading} onChange={(event) => void sortBy(event.target.value as EntrySort)}>
                            {(sortOptions ?? entrySortOptions).map((option) => <NativeSelectOption key={option.value} value={option.value}>{option.label}</NativeSelectOption>)}
                        </NativeSelect>
                    </label>
                )}
                <span className="file-selection-actions">
                    <Button variant="outline" size="sm" disabled={loading || selection.size === 0} onClick={() => removeEntries(selection)}>
                        Remove from list
                    </Button>
                    {variant !== 'workspace' && <Button variant="ghost" size="sm" disabled={loading} onClick={clearFiles} className="file-selection-clear">
                        Clear list
                    </Button>}
                </span>
            </div>
            <p className="file-selection-hint">{listHint}</p>
            {flagged.length > 0 && (
                <div className="file-selection-flagged">
                    <p className="workspace-note">
                        {intakeCopy
                            ? intakeCopy.flaggedNotice(flagged.length, flagged.filter((status) => status.flag?.kind === 'willSkip').length)
                            : `${flagged.length} ${flagged.length === 1 ? "file won't" : "files won't"} run with ${utility.title}. ${flagged.length === 1 ? 'It stays' : 'They stay'} in the list for other actions.`}
                    </p>
                    <Button variant="outline" size="sm" disabled={loading} onClick={() => removeEntries(new Set(flagged.map((status) => status.entry.key)))}>
                        Remove {flagged.length} flagged {flagged.length === 1 ? 'file' : 'files'}
                    </Button>
                </div>
            )}
            <Card className="file-selection-list max-h-60 overflow-y-auto py-0">
                <CardContent className="file-selection-list-content p-1">
                    <ul
                        className="file-selection-rows"
                        role="listbox"
                        aria-multiselectable="true"
                        aria-label={describesEntries ? 'Merge order, top joins first' : ordered ? 'Files, in order' : 'Files'}
                        aria-disabled={loading || undefined}
                        onKeyDown={listKeyDown}
                    >
                        {statuses.map(({ entry, occurrence, flag }, index) => {
                            const pasted = pastedHtml(entry.path);
                            const name = pasted?.name ?? fileName(entry.path);
                            const isSelected = selection.has(entry.key);
                            const facts = describesEntries ? entryFacts?.(entry.path) : undefined;
                            const pages = facts?.pageCount != null ? pageCountLabel(facts.pageCount) : null;
                            const copies = copiesByPath.get(entry.path) ?? 1;
                            const folder = shortenHomePath(parentFolder(entry.path), home);
                            const detail = facts?.modifiedMs != null ? `${folder} · Modified ${modifiedLabel(facts.modifiedMs)}` : folder;
                            const mark = flag ? null : intakeCopy?.mark?.(entry.path) ?? null;
                            const label = mark ? `${name}, ${mark.toLowerCase()}` : name;
                            const rowLabel = describesEntries
                                ? [`${index + 1}. ${label}`, pages].filter(Boolean).join(', ')
                                : ordered ? `${index + 1} of ${entries.length}, ${label}` : label;
                            const warning = flag ? null : fileWarning?.(entry.path) ?? null;
                            return (
                                <li
                                    key={entry.key}
                                    role="option"
                                    aria-selected={isSelected}
                                    aria-label={rowLabel}
                                    data-intake-key={entry.key}
                                    data-flag={flag?.kind}
                                    data-dragging={dragKey === entry.key ? 'true' : undefined}
                                    data-drop-target={dropTargetKey === entry.key && dragKey !== entry.key ? 'true' : undefined}
                                    className="file-selection-row"
                                >
                                    {ordered && (
                                        <Button
                                            variant="ghost"
                                            size="icon-sm"
                                            className="file-selection-handle"
                                            aria-label={`Drag ${name} to reorder`}
                                            disabled={loading}
                                            onPointerDown={(event) => {
                                                event.currentTarget.setPointerCapture(event.pointerId);
                                                setDragKey(entry.key);
                                            }}
                                            onPointerMove={(event) => {
                                                if (dragKey) setDropTargetKey(dropTargetAt(event.clientX, event.clientY));
                                            }}
                                            onPointerUp={endDrag}
                                            onPointerCancel={() => {
                                                setDragKey(null);
                                                setDropTargetKey(null);
                                            }}
                                        >
                                            <GripVertical aria-hidden="true" />
                                        </Button>
                                    )}
                                    {ordered && <span className="file-selection-index" aria-hidden="true">{index + 1}</span>}
                                    <Button
                                        ref={(element) => {
                                            if (element) rowButtons.current.set(entry.key, element);
                                            else rowButtons.current.delete(entry.key);
                                        }}
                                        variant={isSelected ? 'secondary' : 'ghost'}
                                        size="sm"
                                        className="file-selection-item"
                                        data-selected={isSelected ? 'true' : undefined}
                                        aria-pressed={isSelected}
                                        disabled={loading}
                                        onClick={(event) => {
                                            event.stopPropagation();
                                            clickRow(entry.key, event);
                                        }}
                                    >
                                        <span className="file-selection-name">{name}</span>
                                        <span className="file-selection-detail" data-tone={warning ? 'warning' : undefined}>{flag?.message ?? warning ?? (pasted ? pastedRowDetail(pasted) : detail)}</span>
                                    </Button>
                                    {describesEntries && copies > 1 && <Badge variant="secondary" className="file-selection-badge">Copy {occurrence} of {copies}</Badge>}
                                    {pages && <span className="file-selection-fact">{pages}</span>}
                                    {facts?.sizeBytes != null && <span className="file-selection-fact">{fileSizeLabel(facts.sizeBytes)}</span>}
                                    {flag && <Badge variant="outline" className="file-selection-badge">{flag.label ?? (flag.kind === 'willSkip' ? 'Will skip' : "Won't run")}</Badge>}
                                    {mark && <Badge variant="outline" className="file-selection-badge">{mark}</Badge>}
                                    {!flag && renderFileExtra?.(entry.path, isSelected, loading)}
                                    {ordered && !describesEntries && occurrence > 1 && <Badge variant="outline" className="file-selection-badge" data-tone="warning">{repeatLabel(occurrence)}</Badge>}
                                    {ordered && (
                                        <span className="file-selection-move">
                                            <Button variant="ghost" size="icon-sm" aria-label={`Move ${name} up`} disabled={loading || index === 0} onClick={() => moveTo(entry.key, index - 1)}>
                                                <ArrowUp aria-hidden="true" />
                                            </Button>
                                            <Button variant="ghost" size="icon-sm" aria-label={`Move ${name} down`} disabled={loading || index === entries.length - 1} onClick={() => moveTo(entry.key, index + 1)}>
                                                <ArrowDown aria-hidden="true" />
                                            </Button>
                                        </span>
                                    )}
                                    <Button variant="ghost" size="icon-sm" aria-label={`Remove ${name} from list`} disabled={loading} onClick={() => removeEntries(new Set([entry.key]))}>
                                        <XIcon aria-hidden="true" />
                                    </Button>
                                </li>
                            );
                        })}
                    </ul>
                </CardContent>
            </Card>
            {moveStatus && <p className="file-selection-status" role="status">{moveStatus}</p>}
        </div>
    );

    const notAddedNotice = notAdded.length > 0 && (
        <Card role="region" aria-label="Not added" className="intake-not-added py-0">
            <CardContent className="intake-not-added-content p-3">
                <div className="intake-not-added-header">
                    <strong>{notAdded.length} {notAdded.length === 1 ? 'item' : 'items'} not added</strong>
                    <Button variant="ghost" size="sm" onClick={() => setNotAdded([])}>Dismiss</Button>
                </div>
                <ul className="intake-not-added-rows">
                    {notAdded.map((item, index) => (
                        <li key={`${item.path}-${index}`} data-reason={item.reason}>
                            {item.reason === 'folder' ? <FolderIcon aria-hidden="true" /> : <FileIcon aria-hidden="true" />}
                            <span className="intake-not-added-name">{fileName(item.path)}</span>
                            <span className="intake-not-added-message">{item.message}</span>
                            {item.redirectTo && (
                                <Button variant="outline" size="sm" onClick={() => item.redirectTo && followPointer({ kind: 'tool', toolId: item.redirectTo }, item.path)}>
                                    {UtilityRegistry.find((tool) => tool.id === item.redirectTo)?.title}
                                </Button>
                            )}
                        </li>
                    ))}
                </ul>
            </CardContent>
        </Card>
    );

    const workspaceSource = (
        <Card role="region" className="workspace-source-bar py-0" aria-label="Open document" data-empty={entries.length === 0 ? 'true' : 'false'}>
            <CardContent className="workspace-source-content py-3">
            <div className="workspace-source-copy">
                <span className="workspace-source-kicker">Source</span>
                <strong>{entries.length ? `${entries.length} ${entries.length === 1 ? 'file' : 'files'} open` : 'Open a file to begin'}</strong>
                <span>{entries.length ? renderReport ? 'Nothing is changed or saved. Your original stays unchanged.' : 'Export saves a new copy. Your original stays unchanged.' : emptyHint}</span>
            </div>
            <div className="workspace-source-actions">
                <Button variant="default" size="sm" className="workspace-source-open" aria-label="Choose files to process" disabled={loading} {...shortcutTarget('open')} onClick={() => void browse()}>
                    <TablerIcon name="folder-open" />
                    {entries.length ? inputPolicy.inputCardinality === 'single' ? 'Replace file' : 'Add files' : 'Open files'}
                    <ShortcutHint id="open" />
                </Button>
                {pasteButton}
                {entries.length > 0 && <Button variant="ghost" size="sm" className="workspace-source-clear" disabled={loading} onClick={clearFiles}>Clear list</Button>}
            </div>
            {entries.length > 0 && fileSelection}
            {notAddedNotice}
            </CardContent>
        </Card>
    );

    return (
        <div className={`tool-scaffold ${variant === 'workspace' ? 'tool-scaffold-workspace' : ''}`}>
            {variant === 'workspace' ? workspaceSource : <>
                <div className="tool-scaffold-heading">
                    <h2>{utility.title}</h2>
                    <p>{utility.blurb}</p>
                </div>

                <Card role="region" aria-label="Input files" className="file-dropzone flex-1 border-dashed">
                    <CardContent className="file-dropzone-content flex flex-1 flex-col items-center justify-center gap-6 p-6">
                    <div className="file-dropzone-copy">
                        <div className="file-dropzone-icon" aria-hidden="true">
                            <TablerIcon name="file-minus" className="file-dropzone-icon-glyph" />
                        </div>
                        <p>Drag & Drop files here</p>
                        <p>or</p>
                        <Button variant="ghost" size="sm" disabled={loading} {...shortcutTarget('open')} onClick={() => void browse()}>
                            <TablerIcon name="folder-open" /> Browse files
                            <ShortcutHint id="open" />
                        </Button>
                        {pasteButton}
                    </div>

                    {entries.length > 0 && fileSelection}
                    {notAddedNotice}
                    </CardContent>
                </Card>
            </>}

            <fieldset className="tool-scaffold-options" disabled={loading} aria-busy={loading || undefined}>
            {children({
                files,
                failingFiles,
                outputLocation,
                run,
                runCombined,
                runSavingAs,
                loading,
                progress,
                selectedFileIndex,
                selectFile,
                fileKeys,
                moveFile,
                removeFile,
                destination: (
                    <>
                    <RunDestination
                        location={outputLocation}
                        defaultLocation={defaultLocation}
                        overridden={runOverride !== null}
                        toolTitle={utility.title}
                        label={destinationCopy?.(files).label}
                        note={destinationCopy?.(files).note}
                        exampleName={destinationCopy ? null : files[0] && !savesOneFile ? (exampleName ?? ((path) => exampleOutputName(path, utility.command, inputPolicy.outputSuffix)))(files[0]) : null}
                        exampleHint={exampleHint}
                        savesOneFile={saveStep !== null && (files.length === 0 || savesOneFile)}
                        mentionsSettingKept={destinationMentionsSettingKept}
                        onChange={() => void changeForRun()}
                        onReset={() => setRunOverride(null)}
                    />
                    {hasPasted && <p className="workspace-note">Pasted HTML has no original, so it asks where to save when you start.</p>}
                    </>
                ),
                runDestination: {
                    location: outputLocation,
                    defaultLocation,
                    overridden: runOverride !== null,
                    change: () => void changeForRun(),
                    reset: () => setRunOverride(null),
                },
                live,
                runLabel: savesOneFile ? saveStep.runLabel : hasPasted ? `Export ${utility.shortTitle}…` : null,
                failedFiles,
                retryFailed,
                retrying,
                browse,
                replaceFiles,
            })}
            </fieldset>

            {loading && live && !inputPolicy.manyToOne && <CancelRun key={live.runId} run={live} lockedNote={utility.lockedNote} />}

            {renderReport && !loading && results.length > 0 ? renderReport(results, {
                withPassword: perFileRun && retryWithPassword ? (inputPath, password) => rerun([inputPath], retryWithPassword(inputPath, password)) : undefined,
                destination: { location: outputLocation, change: () => void changeForRun() },
                saveTo,
            }) : <ResultList
                results={results}
                progress={progress}
                loading={loading}
                resultVerb={utility.resultVerb}
                resultNoun={utility.resultNoun}
                runningVerb={utility.runningVerb}
                runningDetail={utility.runningDetail}
                progressByPage={utility.progressByPage}
                signatureRemovedNote={utility.signatureRemovedNote}
                precisionReducedIsNote={utility.precisionReducedIsNote}
                live={live}
                combinedInputs={combinedInputs}
                combined={saveStep?.combined}
                outputNoun={utility.outputNoun}
                onRetry={perFileRun ? (inputPath) => rerun([inputPath]) : undefined}
                onRetryWithPassword={perFileRun && retryWithPassword ? (inputPath, password) => rerun([inputPath], retryWithPassword(inputPath, password)) : undefined}
                outputGroup={outputGroup}
                resultsHint={resultsHint}
            />}

            {pasteHtml && <PasteHtmlDialog open={pasting} onAdd={addPasted} onClose={() => setPasting(false)} />}

            <BlockedDestinationDialog
                blocked={blocked}
                onCancel={() => setBlocked(null)}
                onSaveAlongside={saveBlockedAlongside}
                onChooseAnother={() => void chooseBlockedFolder()}
            />
        </div>
    );
};
