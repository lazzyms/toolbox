import { useState, useEffect, useMemo, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open } from '@tauri-apps/plugin-dialog';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import type { ToolDefinition, JobOutcome, OutputLocation, Progress } from '../contracts';
import { useOutputLocation } from '../outputLocation';
import { failedOutcome, parseToolResult } from '../jobOutcome';
import { toolsForWorkspace, workspaceForTool } from '../registry';
import { shortenHomePath, useHomeDir } from '../homePath';
import { ResultList } from './ResultList';
import { BATCH_PROGRESS_EVENT, foldRunEvent, parseRunEvent, startRun, type LiveRun } from './liveRun';
import { BlockedDestinationDialog, RunDestination, exampleOutputName, sameLocation, type BlockedDestination, type FolderCheck } from './RunDestination';
import { TablerIcon } from './TablerIcon';
import { chooseManyToOneSavePath } from './saveStep';
import { saveStepAppliesTo, saveStepFor } from '../registry/saveSteps';
import { ArrowDown, ArrowUp, FileIcon, FolderIcon, GripVertical, XIcon } from 'lucide-react';
import {
    classifyIntake,
    entryStatuses,
    fileName,
    intakePolicyFor,
    moveEntry,
    parentFolder,
    parseInspection,
    rangeKeys,
    type InspectedPath,
    type IntakeEntry,
    type NotAdded,
} from './intake';

const isWindows = () => typeof navigator !== 'undefined' && navigator.platform.toLowerCase().includes('win');

const repeatLabel = (occurrence: number) => occurrence === 2 ? 'Added twice' : `Added ${occurrence} times`;

export type WorkspaceSourceAction = () => void | Promise<void>;

export type RunContext = { outputPath: string | null; runId: string };

export type RunHandler = (files: string[], outputLocation: OutputLocation, run: RunContext) => Promise<JobOutcome[]>;

type PerFileRun = { handler: RunHandler; outputLocation: OutputLocation };

interface ToolScaffoldProps {
    utility: ToolDefinition;
    onRun: RunHandler;
    onRunCombined?: RunHandler;
    variant?: 'standard' | 'workspace';
    sessionKey?: string;
    onWorkspaceSourceAction?: (action: WorkspaceSourceAction | null) => void;
    showFileOrdering?: boolean;
    renderFileExtra?: (path: string, selected: boolean, locked: boolean) => React.ReactNode;
    retryWithPassword?: (password: string) => RunHandler;
    children: (props: {
        files: string[];
        outputLocation: OutputLocation;
        run: () => Promise<void>;
        runCombined: () => Promise<void>;
        loading: boolean;
        progress: Progress;
        selectedFileIndex: number;
        selectFile: (index: number) => void;
        destination: React.ReactNode;
        runLabel: string | null;
    }) => React.ReactNode;
}

export const ToolScaffold = ({ utility, onRun, onRunCombined, variant = 'standard', sessionKey, onWorkspaceSourceAction, showFileOrdering = true, renderFileExtra, retryWithPassword, children }: ToolScaffoldProps) => {
    const [entries, setEntries] = useState<IntakeEntry[]>([]);
    const [selection, setSelection] = useState<ReadonlySet<string>>(() => new Set());
    const [anchor, setAnchor] = useState<string | null>(null);
    const [notAdded, setNotAdded] = useState<NotAdded[]>([]);
    const [moveStatus, setMoveStatus] = useState('');
    const [dragKey, setDragKey] = useState<string | null>(null);
    const [dropTargetKey, setDropTargetKey] = useState<string | null>(null);
    const nextEntryKey = useRef(0);
    const rowButtons = useRef(new Map<string, HTMLButtonElement>());
    const home = useHomeDir();
    const [results, setResults] = useState<JobOutcome[]>([]);
    const [resultsNamedByOutput, setResultsNamedByOutput] = useState(false);
    const [loading, setLoading] = useState(false);
    const [live, setLive] = useState<LiveRun | null>(null);
    const [perFileRun, setPerFileRun] = useState<PerFileRun | null>(null);
    const defaultLocation = useOutputLocation();
    const [runOverride, setRunOverride] = useState<OutputLocation | null>(null);
    const [blocked, setBlocked] = useState<(BlockedDestination & { handler: RunHandler }) | null>(null);
    const outputLocation = runOverride ?? defaultLocation;
    const runGeneration = useRef(0);
    const browseRef = useRef<WorkspaceSourceAction>(() => undefined);
    const inputPolicy = utility.capability;
    const workspace = variant === 'workspace' ? workspaceForTool(utility.id) : undefined;
    const intakeTools = useMemo(() => workspace ? toolsForWorkspace(workspace) : [utility], [workspace, utility]);
    const ordered = inputPolicy.inputCardinality === 'ordered' && showFileOrdering;
    const statuses = useMemo(() => entryStatuses(entries, utility), [entries, utility]);
    const runnable = useMemo(() => statuses.filter((status) => status.wontRun === null), [statuses]);
    const files = useMemo(() => runnable.map((status) => status.entry.path), [runnable]);
    const flagged = statuses.filter((status) => status.wontRun !== null);
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

    const runStreamed = async (handler: RunHandler, paths: string[], location: OutputLocation, outputPath: string | null, generation: number) => {
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
            return parseToolResult(await handler(paths, location, { outputPath, runId }));
        } finally {
            unlisten?.();
        }
    };

    const runWith = async (handler: RunHandler, location: OutputLocation = outputLocation) => {
        const generation = ++runGeneration.current;
        setLoading(true);
        setResults([]);
        setResultsNamedByOutput(false);
        setPerFileRun(null);
        try {
            if (location !== 'alongsideInput') {
                const check = await invoke<FolderCheck>('check_output_folder', { folder: location.customFolder });
                if (check !== 'ready') {
                    if (generation === runGeneration.current) setBlocked({ folder: location.customFolder, check, handler });
                    return;
                }
            }
            if (!inputPolicy.manyToOne) setPerFileRun({ handler, outputLocation: location });
            const cardinalityError = inputPolicy.inputCardinality === 'single' && files.length > 1
                ? `This action accepts one input file, but ${files.length} were selected.`
                : null;
            const availabilityError = inputPolicy.nativeAvailability === 'unavailable'
                ? `${utility.title} is unavailable in this build.`
                : null;
            const invalidPaths = new Map<string, JobOutcome>();
            const validPaths = files.filter((path) => {
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
            const orderedSelectionRejected = inputPolicy.inputCardinality === 'ordered' && invalidPaths.size > 0;
            const runs = !orderedSelectionRejected && validPaths.length > 0;
            let outputPath: string | null = null;
            if (runs && saveStepAppliesTo(saveStep, validPaths.length)) {
                setLoading(false);
                outputPath = await chooseManyToOneSavePath(saveStep, validPaths, location, inputPolicy.outputSuffix, saveStep.summary(validPaths.length));
                if (!outputPath || generation !== runGeneration.current) return;
                setLoading(true);
            }
            const processed = runs ? await runStreamed(handler, validPaths, location, outputPath, generation) : [];
            const processedByPath = new Map(processed.map((result) => [result.inputPath, result]));
            const nextResults = orderedSelectionRejected
                ? files.map((path) => invalidPaths.get(path) ?? failedOutcome(path, 'invalidInput', 'Every selected input must use a supported format for this ordered action.'))
                : invalidPaths.size === 0
                    ? processed
                : files.map((path) => invalidPaths.get(path) ?? processedByPath.get(path)).filter((result): result is JobOutcome => result !== undefined);
            if (generation === runGeneration.current) {
                setResults(nextResults);
                setResultsNamedByOutput(outputPath !== null);
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
        void runWith(blocked.handler, 'alongsideInput');
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
        void runWith(blocked.handler, location);
    };

    const retry = async (inputPath: string, handler = perFileRun?.handler) => {
        if (!perFileRun || !handler) return;
        const generation = runGeneration.current;
        let retried: JobOutcome[];
        try {
            retried = parseToolResult(await handler([inputPath], perFileRun.outputLocation, { outputPath: null, runId: crypto.randomUUID() }));
        } catch (error) {
            retried = [failedOutcome(inputPath, 'processing', String(error))];
        }
        const replacement = retried.find((result) => result.inputPath === inputPath);
        if (!replacement || generation !== runGeneration.current) return;
        setResults((current) => current.map((result) => result.inputPath === inputPath ? replacement : result));
    };

    const newEntries = (paths: string[]) => paths.map((path) => ({ key: `entry-${++nextEntryKey.current}`, path }));

    const selectOnly = (key: string | null) => {
        setSelection(new Set(key ? [key] : []));
        setAnchor(key);
    };

    const intake = (incoming: InspectedPath[], replaceSingle = false) => {
        if (inputPolicy.inputCardinality === 'single' && replaceSingle) {
            const next = incoming[0]?.path;
            if (!next || (entries.length === 1 && entries[0].path === next)) return;
            const [entry] = newEntries([next]);
            setNotAdded([]);
            setEntries([entry]);
            selectOnly(entry.key);
            return;
        }
        const policy = intakePolicyFor(workspace?.title ?? utility.title, intakeTools, utility);
        const classified = classifyIntake(entries.map((entry) => entry.path), incoming, policy);
        setNotAdded(classified.notAdded);
        if (classified.added.length) setEntries((current) => [...current, ...newEntries(classified.added)]);
    };

    const dropPaths = async (paths: string[]) => {
        let inspected: InspectedPath[];
        try {
            inspected = parseInspection(await invoke('inspect_intake_paths', { paths }), paths);
        } catch {
            inspected = parseInspection(null, paths);
        }
        intake(inspected);
    };

    const dropRef = useRef<(paths: string[]) => void>(() => undefined);
    dropRef.current = (paths) => {
        if (!loading) void dropPaths(paths);
    };

    useEffect(() => {
        if (typeof window === 'undefined' || !("__TAURI_INTERNALS__" in window)) return;
        const unlisten = getCurrentWebviewWindow().onDragDropEvent((event) => {
            if (event.payload.type === 'drop') dropRef.current(event.payload.paths);
        });
        return () => {
            unlisten.then((dispose) => dispose());
        };
    }, []);

    const browse = async () => {
        if (loading) return;
        try {
            const picked = await open({
                multiple: inputPolicy.inputCardinality !== 'single',
                filters: [{ name: 'Supported files', extensions: inputPolicy.acceptedExtensions.map((extension) => extension.replace(/^\./, '')) }],
            });
            const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
            if (paths.length) intake(parseInspection(null, paths), true);
        } catch (e) {
            setResults([failedOutcome('Dialog error', 'processing', String(e))]);
        }
    };

    browseRef.current = browse;
    useEffect(() => {
        if (variant !== 'workspace' || !onWorkspaceSourceAction) return;
        onWorkspaceSourceAction(entries.length === 0 ? () => browseRef.current() : null);
        return () => onWorkspaceSourceAction(null);
    }, [entries.length, onWorkspaceSourceAction, variant]);

    const removeEntries = (keys: ReadonlySet<string>) => {
        if (loading || keys.size === 0) return;
        setEntries((current) => current.filter((entry) => !keys.has(entry.key)));
        selectOnly(null);
        setMoveStatus('');
    };

    const clearFiles = () => {
        if (loading) return;
        setEntries([]);
        setResults([]);
        setNotAdded([]);
        setMoveStatus('');
        selectOnly(null);
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

    const moveTo = (key: string, toIndex: number) => {
        if (loading) return;
        const next = moveEntry(entries, key, toIndex);
        if (next === entries) return;
        setEntries(next);
        selectOnly(key);
        const moved = next[toIndex];
        setMoveStatus(`Moved ${fileName(moved.path)} to position ${toIndex + 1}.`);
        focusRow(key);
    };

    const listKeyDown = (event: React.KeyboardEvent<HTMLUListElement>) => {
        if (loading) return;
        if (event.key === 'Delete' || event.key === 'Backspace') {
            event.preventDefault();
            removeEntries(selection);
            return;
        }
        if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
        const index = entries.findIndex((entry) => entry.key === anchor);
        if (index < 0) return;
        const delta = event.key === 'ArrowUp' ? -1 : 1;
        event.preventDefault();
        if (event.metaKey || event.ctrlKey) {
            if (ordered) moveTo(entries[index].key, index + delta);
            return;
        }
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
    const listHint = ordered
        ? `Files run top to bottom. Drag a row, or select it and press ${modifier}+Up or ${modifier}+Down.`
        : `Shift-click or ${isWindows() ? 'Ctrl-click' : '⌘-click'} to select. Delete removes files from the list, not from disk.`;

    const fileSelection = (
        <div className="file-selection" data-locked={loading ? 'true' : undefined}>
            <div className="file-selection-header">
                <span className="file-selection-count">{entries.length} {entries.length === 1 ? 'file' : 'files'}</span>
                {selection.size > 0 && <span className="file-selection-selected">{selection.size} selected</span>}
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
                        {flagged.length} {flagged.length === 1 ? "file won't" : "files won't"} run with {utility.title}. {flagged.length === 1 ? 'It stays' : 'They stay'} in the list for other actions.
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
                        aria-label={ordered ? 'Files, in order' : 'Files'}
                        aria-disabled={loading || undefined}
                        onKeyDown={listKeyDown}
                    >
                        {statuses.map(({ entry, occurrence, wontRun }, index) => {
                            const name = fileName(entry.path);
                            const isSelected = selection.has(entry.key);
                            return (
                                <li
                                    key={entry.key}
                                    role="option"
                                    aria-selected={isSelected}
                                    aria-label={ordered ? `${index + 1} of ${entries.length}, ${name}` : name}
                                    data-intake-key={entry.key}
                                    data-wont-run={wontRun ? 'true' : undefined}
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
                                        <span className="file-selection-detail">{wontRun ?? shortenHomePath(parentFolder(entry.path), home)}</span>
                                    </Button>
                                    {wontRun && <Badge variant="outline" className="file-selection-badge">Won't run</Badge>}
                                    {!wontRun && renderFileExtra?.(entry.path, isSelected, loading)}
                                    {ordered && occurrence > 1 && <Badge variant="outline" className="file-selection-badge" data-tone="warning">{repeatLabel(occurrence)}</Badge>}
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
                <span>{entries.length ? 'Export saves a new copy. Your original stays unchanged.' : 'Drop files here or browse from this device.'}</span>
            </div>
            <div className="workspace-source-actions">
                <Button variant="default" size="sm" className="workspace-source-open" aria-label="Choose files to process" disabled={loading} onClick={() => void browse()}>
                    <TablerIcon name="folder-open" />
                    {entries.length ? inputPolicy.inputCardinality === 'single' ? 'Replace file' : 'Add files' : 'Open files'}
                </Button>
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
                        <Button variant="ghost" size="sm" disabled={loading} onClick={() => void browse()}>
                            <TablerIcon name="folder-open" /> Browse files
                        </Button>
                    </div>

                    {entries.length > 0 && fileSelection}
                    {notAddedNotice}
                    </CardContent>
                </Card>
            </>}

            <fieldset className="tool-scaffold-options" disabled={loading} aria-busy={loading || undefined}>
            {children({
                files,
                outputLocation,
                run,
                runCombined,
                loading,
                progress,
                selectedFileIndex,
                selectFile,
                destination: (
                    <RunDestination
                        location={outputLocation}
                        defaultLocation={defaultLocation}
                        overridden={runOverride !== null}
                        toolTitle={utility.title}
                        exampleName={files[0] && !savesOneFile ? exampleOutputName(files[0], inputPolicy.outputSuffix) : null}
                        savesOneFile={saveStep !== null && (files.length === 0 || savesOneFile)}
                        onChange={() => void changeForRun()}
                        onReset={() => setRunOverride(null)}
                    />
                ),
                runLabel: savesOneFile ? saveStep.runLabel : null,
            })}
            </fieldset>

            <ResultList
                results={results}
                progress={progress}
                loading={loading}
                resultVerb={utility.resultVerb}
                runningVerb={utility.runningVerb}
                live={live}
                namedByOutput={resultsNamedByOutput}
                onRetry={perFileRun ? (inputPath) => retry(inputPath) : undefined}
                onRetryWithPassword={perFileRun && retryWithPassword ? (inputPath, password) => retry(inputPath, retryWithPassword(password)) : undefined}
            />

            <BlockedDestinationDialog
                blocked={blocked}
                onCancel={() => setBlocked(null)}
                onSaveAlongside={saveBlockedAlongside}
                onChooseAnother={() => void chooseBlockedFolder()}
            />
        </div>
    );
};
