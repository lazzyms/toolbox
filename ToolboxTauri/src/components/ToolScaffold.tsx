import { useState, useEffect, useCallback } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open } from '@tauri-apps/plugin-dialog';
import { selectionCountError, type ToolDefinition, type JobOutcome, type Progress } from '../contracts';
import { ResultList } from './ResultList';
import { TablerIcon } from './TablerIcon';

interface ToolScaffoldProps {
    utility: ToolDefinition;
    onRun: (files: string[]) => Promise<JobOutcome[]>;
    children: (props: {
        files: string[];
        run: () => Promise<void>;
        loading: boolean;
        progress: Progress;
    }) => React.ReactNode;
}

export const ToolScaffold = ({ utility, onRun, children }: ToolScaffoldProps) => {
    const [files, setFiles] = useState<string[]>([]);
    const [results, setResults] = useState<JobOutcome[]>([]);
    const [loading, setLoading] = useState(false);
    const progress: Progress = { completed: loading ? 0 : results.length, total: files.length };

    const selectionError = selectionCountError(utility, files.length);
    const addFiles = useCallback((selected: string[]) => {
        const accepted = selected.filter((path) => {
            const extension = path.split(/[\\/.]/).pop()?.toLowerCase();
            return extension !== undefined && utility.acceptedExtensions.includes(extension);
        });
        const rejected = selected.filter((path) => !accepted.includes(path));
        if (rejected.length > 0) {
            setResults(rejected.map((inputPath) => ({
                inputPath,
                outputPaths: [],
                detail: '',
                failure: { kind: 'invalidInput', message: `Unsupported file type for ${utility.shortTitle}. Accepted types: ${utility.acceptedExtensions.map((extension) => `.${extension}`).join(', ')}.` },
            })));
        }
        const fresh = accepted.filter((path) => !files.includes(path));
        if (utility.inputCardinality.kind === 'single' && files.length + fresh.length > 1) {
            setResults([{ inputPath: fresh[0] ?? files[0] ?? '', outputPaths: [], detail: '', failure: { kind: 'invalidInput', message: `${utility.shortTitle} accepts exactly one file.` } }]);
            return;
        }
        setFiles((previous) => {
            const existing = new Set(previous);
            const newPaths = accepted.filter((path) => !existing.has(path));
            return newPaths.length ? [...previous, ...newPaths] : previous;
        });
    }, [files, utility]);

    const run = async () => {
        if (selectionError) {
            setResults(files.length > 0
                ? files.map((inputPath) => ({ inputPath, outputPaths: [], detail: '', failure: { kind: 'invalidInput', message: selectionError } }))
                : [{ inputPath: '', outputPaths: [], detail: '', failure: { kind: 'invalidInput', message: selectionError } }]);
            return;
        }
        setLoading(true);
        setResults([]);
        try {
            setResults(await onRun(files));
        } catch (error) {
            setResults([{ inputPath: 'Tool error', outputPaths: [], failure: { kind: 'processing', message: String(error) }, detail: '' }]);
        } finally {
            setLoading(false);
        }
    };

    useEffect(() => {
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
    }, [addFiles]);

    const browse = async () => {
        try {
            const picked = await open({
                multiple: utility.inputCardinality.kind === 'multiple',
                filters: [{ name: 'Supported files', extensions: [...utility.acceptedExtensions] }],
            });
            addFiles(Array.isArray(picked) ? picked : picked ? [picked] : []);
        } catch (e) {
            setResults([{ inputPath: 'Dialog error', outputPaths: [], failure: { kind: 'processing', message: String(e) }, detail: '' }]);
        }
    };

    return (
        <div className="tool-scaffold">
            <div className="tool-scaffold-heading">
                <h2>{utility.title}</h2>
                <p>{utility.blurb}</p>
                <p className="text-xs text-slate-500" aria-label="Tool capability">
                    {utility.inputCardinality.kind === 'single' ? 'One file' : `Multiple files, at least ${utility.inputCardinality.minimum}`}.
                    {' '}Accepted: {utility.acceptedExtensions.map((extension) => `.${extension}`).join(', ')}.
                    {utility.supportsPageSelection ? ' Page selection supported.' : ''}
                    {utility.supportsPreview ? ' Output preview supported.' : ''}
                </p>
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

                {files.length > 0 && (
                    <div className="file-selection">
                        <div className="file-selection-header">
                            <span className="file-selection-count">{files.length} files selected</span>
                            <button
                                onClick={(e) => {
                                    e.stopPropagation();
                                    setFiles([]);
                                    setResults([]);
                                }}
                                className="file-selection-clear"
                            >
                                Clear all
                            </button>
                        </div>
                        <div className="file-selection-list">
                            {files.map((f) => (
                                <div key={f} className="file-selection-item">
                                    {f.split(/[\\/]/).pop()}
                                </div>
                            ))}
                        </div>
                    </div>
                )}
            </div>

            {selectionError && files.length > 0 && <p className="text-sm text-amber-700" role="alert">{selectionError}</p>}

            {children({
                files,
                run,
                loading,
                progress,
            })}

            <ResultList results={results} progress={progress} loading={loading} />
        </div>
    );
};
