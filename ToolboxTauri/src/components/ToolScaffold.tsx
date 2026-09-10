import { useState, useEffect, useRef } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open } from '@tauri-apps/plugin-dialog';
import type { ToolDefinition, JobOutcome, Progress } from '../contracts';
import { ResultList } from './ResultList';
import { TablerIcon } from './TablerIcon';

interface ToolScaffoldProps {
    utility: ToolDefinition;
    onRun: (files: string[]) => Promise<JobOutcome[]>;
    onRunCombined?: (files: string[]) => Promise<JobOutcome[]>;
    variant?: 'standard' | 'workspace';
    sessionKey?: string;
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

export const ToolScaffold = ({ utility, onRun, onRunCombined, variant = 'standard', sessionKey, children }: ToolScaffoldProps) => {
    const [files, setFiles] = useState<string[]>([]);
    const [results, setResults] = useState<JobOutcome[]>([]);
    const [loading, setLoading] = useState(false);
    const [selectedFileIndex, setSelectedFileIndex] = useState(0);
    const runGeneration = useRef(0);
    const inputPolicy = utility.capability;
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
            if (inputPolicy.inputCardinality === 'single' && replaceSingle) return fresh;
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

    useEffect(() => {
        if (typeof window === 'undefined' || !("__TAURI_INTERNALS__" in window)) return;
        const appWindow = getCurrentWebviewWindow();
        const unlisten = appWindow.onDragDropEvent((event) => {
            if (event.payload.type === 'drop') {
                const paths = (event.payload as { type: 'drop'; paths: string[] }).paths;
                addFiles(paths);
            }
        });
        return () => {
            unlisten.then((dispose) => dispose());
        };
    }, [inputPolicy.inputCardinality]);

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
                {inputPolicy.inputCardinality === 'ordered' && (
                    <span className="file-selection-order" aria-label="Selected file ordering">
                        <button
                            type="button"
                            aria-label="Move selected file up"
                            disabled={selectedFileIndex === 0}
                            onClick={(event) => {
                                event.stopPropagation();
                                moveSelectedFile(-1);
                            }}
                        >
                            ↑
                        </button>
                        <button
                            type="button"
                            aria-label="Move selected file down"
                            disabled={selectedFileIndex >= files.length - 1}
                            onClick={(event) => {
                                event.stopPropagation();
                                moveSelectedFile(1);
                            }}
                        >
                            ↓
                        </button>
                    </span>
                )}
                {variant !== 'workspace' && <button type="button" onClick={clearFiles} className="file-selection-clear">
                    Close file{files.length === 1 ? '' : 's'}
                </button>}
            </div>
            <div className="file-selection-list">
                {files.map((f, index) => (
                    <button
                        type="button"
                        key={f}
                        className="file-selection-item"
                        data-selected={selectedFileIndex === index ? 'true' : undefined}
                        onClick={(event) => {
                            event.stopPropagation();
                            setSelectedFileIndex(index);
                        }}
                    >
                        {f.split(/[\\/]/).pop()}
                    </button>
                ))}
            </div>
        </div>
    );

    const workspaceSource = (
        <section className="workspace-source-bar" aria-label="Open document">
            <div className="workspace-source-copy">
                <span className="workspace-source-kicker">Source</span>
                <strong>{files.length ? `${files.length} ${files.length === 1 ? 'file' : 'files'} open` : 'Open a file to begin'}</strong>
                <span>{files.length ? 'Export saves a new copy. Your original stays unchanged.' : 'Drop files here or browse from this device.'}</span>
            </div>
            <div className="workspace-source-actions">
                <button type="button" className="workspace-source-open" aria-label="Choose files to process" onClick={() => void browse()}>
                    <TablerIcon name="folder-open" />
                    {files.length ? inputPolicy.inputCardinality === 'single' ? 'Replace file' : 'Add files' : 'Open files'}
                </button>
                {files.length > 0 && <button type="button" className="workspace-source-clear" onClick={clearFiles}>Close</button>}
            </div>
            {files.length > 0 && fileSelection}
        </section>
    );

    return (
        <div className={`tool-scaffold ${variant === 'workspace' ? 'tool-scaffold-workspace' : ''}`}>
            {variant === 'workspace' ? workspaceSource : <>
                <div className="tool-scaffold-heading">
                    <h2>{utility.title}</h2>
                    <p>{utility.blurb}</p>
                </div>

                <div
                    role="button"
                    tabIndex={0}
                    aria-label="Choose files to process"
                    onClick={browse}
                    onKeyDown={(event) => {
                        if (event.key === 'Enter' || event.key === ' ') {
                            event.preventDefault();
                            void browse();
                        }
                    }}
                    className="file-dropzone"
                >
                    <div className="file-dropzone-copy">
                        <div className="file-dropzone-icon" aria-hidden="true">
                            <TablerIcon name="file-minus" className="file-dropzone-icon-glyph" />
                        </div>
                        <p>Drag & Drop files here</p>
                        <p>or click to browse</p>
                    </div>

                    {files.length > 0 && fileSelection}
                </div>
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
