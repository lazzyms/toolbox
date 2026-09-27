import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { Button } from '@/components/ui/button';
import { Card, CardContent } from '@/components/ui/card';
import type { ToolDefinition, JobOutcome, Progress } from '../contracts';
import type { ShellEvent } from '../hooks/useShellBridge';
import { ResultList } from './ResultList';
import { TablerIcon } from './TablerIcon';

export type WorkspaceSourceAction = () => void | Promise<void>;

interface ToolScaffoldProps {
    utility: ToolDefinition;
    onRun: (files: string[]) => Promise<JobOutcome[]>;
    onRunCombined?: (files: string[]) => Promise<JobOutcome[]>;
    variant?: 'standard' | 'workspace';
    sessionKey?: string;
    onWorkspaceSourceAction?: (action: WorkspaceSourceAction | null) => void;
    showFileOrdering?: boolean;
    initialPaths?: readonly string[];
    fileActivation?: Extract<ShellEvent, { kind: 'files' }>;
    onActivationAccepted?: (activationId: string) => void;
    onFilesChange?: (paths: readonly string[]) => void;
    children: (props: {
        files: string[];
        run: () => Promise<void>;
        runCombined: () => Promise<void>;
        loading: boolean;
        progress: Progress;
        selectedFileIndex: number;
        selectFile: (index: number) => void;
        moveSelectedFile: (delta: -1 | 1) => void;
    }) => React.ReactNode;
}

export const ToolScaffold = ({ utility, onRun, onRunCombined, variant = 'standard', sessionKey, onWorkspaceSourceAction, showFileOrdering = true, initialPaths = [], fileActivation, onActivationAccepted, onFilesChange, children }: ToolScaffoldProps) => {
    const [files, setFiles] = useState<string[]>(() => [...initialPaths]);
    const [results, setResults] = useState<JobOutcome[]>([]);
    const [loading, setLoading] = useState(false);
    const [selectedFileIndex, setSelectedFileIndex] = useState(0);
    const [acceptedActivationId, setAcceptedActivationId] = useState<string | null>(null);
    const runGeneration = useRef(0);
    const browseRef = useRef<WorkspaceSourceAction>(() => undefined);
    const inputPolicy = utility.capability;
    const acceptedExtensions = new Set(inputPolicy.acceptedExtensions.map((extension) => extension.toLowerCase()));
    const unsupportedFiles = files.filter((path) => {
        const name = path.split(/[\\/]/).pop() ?? path;
        const extension = name.includes('.') ? `.${name.split('.').pop()?.toLowerCase()}` : '';
        return !acceptedExtensions.has(extension);
    });
    const inputPolicyIssue = inputPolicy.inputCardinality === 'single' && files.length > 1
        ? `This action accepts one input file, but ${files.length} are open. Close the extra files or switch to a multi-file action.`
        : unsupportedFiles.length > 0
            ? `${unsupportedFiles.length} ${unsupportedFiles.length === 1 ? 'file is' : 'files are'} not supported by ${utility.title}. Close ${unsupportedFiles.length === 1 ? 'it' : 'them'} or switch actions.`
            : null;
    const progress: Progress = {
        completed: loading ? 0 : Math.min(results.length, files.length),
        total: files.length,
    };

    const resolvedSessionKey = sessionKey ?? utility.id;

    useEffect(() => {
        runGeneration.current += 1;
        setResults([]);
        setLoading(false);
        setSelectedFileIndex(0);
    }, [resolvedSessionKey]);

    useEffect(() => {
        runGeneration.current += 1;
        setResults([]);
        setLoading(false);
    }, [utility.id]);

    useEffect(() => {
        runGeneration.current += 1;
        setResults([]);
        setSelectedFileIndex((index) => Math.min(index, Math.max(files.length - 1, 0)));
    }, [files]);

    useEffect(() => {
        if (!fileActivation) return;
        setFiles([...fileActivation.paths]);
        setResults([]);
        setSelectedFileIndex(0);
        setAcceptedActivationId(fileActivation.activationId);
    }, [fileActivation?.activationId]);

    useEffect(() => {
        if (!fileActivation || acceptedActivationId !== fileActivation.activationId) return;
        void invoke('acknowledge_activation', { activationId: fileActivation.activationId }).then(() => {
            onActivationAccepted?.(fileActivation.activationId);
        });
    }, [acceptedActivationId, fileActivation, onActivationAccepted]);

    useEffect(() => {
        onFilesChange?.(files);
    }, [files, onFilesChange]);

    const failureFor = (inputPath: string, kind: 'invalidInput' | 'unavailable', message: string): JobOutcome => ({
        inputPath,
        outputPaths: [],
        detail: '',
        failure: { kind, message },
    });

    const runWith = async (handler: (files: string[]) => Promise<JobOutcome[]>) => {
        const generation = ++runGeneration.current;
        setLoading(true);
        setResults([]);
        try {
            const cardinalityError = inputPolicy.inputCardinality === 'single' && files.length > 1
                ? `This action accepts one input file, but ${files.length} were selected.`
                : null;
            const availabilityError = inputPolicy.nativeAvailability === 'unavailable'
                ? `${utility.title} is unavailable in this build.`
                : null;
            const invalidPaths = new Map<string, JobOutcome>();
            const validPaths = files.filter((path) => {
                if (cardinalityError) {
                    invalidPaths.set(path, failureFor(path, 'invalidInput', cardinalityError));
                    return false;
                }
                if (availabilityError) {
                    invalidPaths.set(path, failureFor(path, 'unavailable', availabilityError));
                    return false;
                }
                return true;
            });
            const orderedSelectionRejected = inputPolicy.inputCardinality === 'ordered' && invalidPaths.size > 0;
            const processed = orderedSelectionRejected || validPaths.length === 0 ? [] : await handler(validPaths);
            const processedByPath = new Map(processed.map((result) => [result.inputPath, result]));
            const nextResults = orderedSelectionRejected
                ? files.map((path) => invalidPaths.get(path) ?? failureFor(path, 'invalidInput', 'Every selected input must use a supported format for this ordered action.'))
                : invalidPaths.size === 0
                    ? processed
                : files.map((path) => invalidPaths.get(path) ?? processedByPath.get(path)).filter((result): result is JobOutcome => result !== undefined);
            if (generation === runGeneration.current) setResults(nextResults);
        } catch (error) {
            if (generation === runGeneration.current) {
                setResults([{ inputPath: 'Tool error', outputPaths: [], failure: { kind: 'processing', message: String(error) }, detail: '' }]);
            }
        } finally {
            if (generation === runGeneration.current) setLoading(false);
        }
    };
    const run = () => runWith(onRun);
    const runCombined = () => runWith(onRunCombined ?? onRun);

    const addFiles = (paths: string[], replaceSingle = false) => {
        setFiles((prev) => {
            const existing = new Set(prev);
            const fresh = paths.filter((p) => !existing.has(p));
            if (inputPolicy.inputCardinality === 'single' && replaceSingle) {
                const next = paths[0];
                return next && next !== prev[0] ? [next] : prev;
            }
            return fresh.length ? [...prev, ...fresh] : prev;
        });
    };

    const moveSelectedFile = (delta: -1 | 1) => {
        setFiles((current) => {
            const nextIndex = selectedFileIndex + delta;
            if (nextIndex < 0 || nextIndex >= current.length) return current;
            const next = [...current];
            [next[selectedFileIndex], next[nextIndex]] = [next[nextIndex], next[selectedFileIndex]];
            setSelectedFileIndex(nextIndex);
            return next;
        });
    };

    const browse = async () => {
        try {
            const picked = await open({
                multiple: inputPolicy.inputCardinality !== 'single',
                filters: inputPolicy.acceptedExtensions.length
                    ? [{ name: 'Supported files', extensions: inputPolicy.acceptedExtensions.map((extension) => extension.replace(/^\./, '')) }]
                    : undefined,
            });
            addFiles(Array.isArray(picked) ? picked : picked ? [picked] : [], inputPolicy.inputCardinality === 'single');
        } catch (e) {
            setResults([{ inputPath: 'Dialog error', outputPaths: [], failure: { kind: 'processing', message: String(e) }, detail: '' }]);
        }
    };

    browseRef.current = browse;
    useEffect(() => {
        if (variant !== 'workspace' || !onWorkspaceSourceAction) return;
        onWorkspaceSourceAction(files.length === 0 ? () => browseRef.current() : null);
        return () => onWorkspaceSourceAction(null);
    }, [files.length, onWorkspaceSourceAction, variant]);

    const clearFiles = (event?: React.MouseEvent<HTMLButtonElement>) => {
        event?.stopPropagation();
        setFiles([]);
        setResults([]);
        setSelectedFileIndex(0);
    };

    const fileSelection = (
        <div className="file-selection">
            <div className="file-selection-header">
                <span className="file-selection-count">{files.length} {files.length === 1 ? 'file' : 'files'} open</span>
                {inputPolicy.inputCardinality === 'ordered' && showFileOrdering && (
                    <span className="file-selection-order" aria-label="Selected file ordering">
                        <Button
                            size="icon-sm"
                            aria-label="Move selected file up"
                            disabled={selectedFileIndex === 0}
                            onClick={(event) => {
                                event.stopPropagation();
                                moveSelectedFile(-1);
                            }}
                        >
                            ↑
                        </Button>
                        <Button
                            size="icon-sm"
                            aria-label="Move selected file down"
                            disabled={selectedFileIndex >= files.length - 1}
                            onClick={(event) => {
                                event.stopPropagation();
                                moveSelectedFile(1);
                            }}
                        >
                            ↓
                        </Button>
                    </span>
                )}
                {variant !== 'workspace' && <Button variant="ghost" size="sm" onClick={clearFiles} className="file-selection-clear">
                    Close file{files.length === 1 ? '' : 's'}
                </Button>}
            </div>
            <Card className="file-selection-list max-h-40 overflow-y-auto py-0">
                <CardContent className="file-selection-list-content flex flex-col gap-1 p-2">
                {files.map((f, index) => (
                    <Button
                        variant={selectedFileIndex === index ? 'secondary' : 'ghost'}
                        size="sm"
                        key={f}
                        className="file-selection-item"
                        data-selected={selectedFileIndex === index ? 'true' : undefined}
                        aria-pressed={selectedFileIndex === index}
                        onClick={(event) => {
                            event.stopPropagation();
                            setSelectedFileIndex(index);
                        }}
                    >
                        {f.split(/[\\/]/).pop()}
                    </Button>
                ))}
                </CardContent>
            </Card>
        </div>
    );

    const workspaceSource = (
        <Card role="region" className="workspace-source-bar py-0" aria-label="Open document" data-empty={files.length === 0 ? 'true' : 'false'}>
            <CardContent className="workspace-source-content py-3">
            <div className="workspace-source-copy">
                <span className="workspace-source-kicker">Source</span>
                <strong>{files.length ? `${files.length} ${files.length === 1 ? 'file' : 'files'} open` : 'Open a file to begin'}</strong>
                <span>{files.length ? 'Export saves a new copy. Your original stays unchanged.' : 'Drop files here or browse from this device.'}</span>
            </div>
            <div className="workspace-source-actions">
                <Button variant="default" size="sm" className="workspace-source-open" aria-label="Choose files to process" onClick={() => void browse()}>
                    <TablerIcon name="folder-open" />
                    {files.length ? inputPolicy.inputCardinality === 'single' ? 'Replace file' : 'Add files' : 'Open files'}
                </Button>
                {files.length > 0 && <Button variant="ghost" size="sm" className="workspace-source-clear" onClick={clearFiles}>Close</Button>}
            </div>
            {files.length > 0 && fileSelection}
            {inputPolicyIssue && <p className="workspace-note" role="alert">{inputPolicyIssue}</p>}
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
                        <Button variant="ghost" size="sm" onClick={() => void browse()}>
                            <TablerIcon name="folder-open" /> Browse files
                        </Button>
                    </div>

                    {files.length > 0 && fileSelection}
                    </CardContent>
                </Card>
            </>}

            {children({
                files,
                run,
                runCombined,
                loading,
                progress,
                selectedFileIndex,
                selectFile: setSelectedFileIndex,
                moveSelectedFile,
            })}

            <ResultList results={results} progress={progress} loading={loading} />
        </div>
    );
};
