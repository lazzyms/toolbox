import { useState, useEffect } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open } from '@tauri-apps/plugin-dialog';
import type { ToolDefinition, JobOutcome, Progress } from '../contracts';
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

    const run = async () => {
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
                // Dedupe: skip paths already in the list
                setFiles((prev) => {
                    const existing = new Set(prev);
                    const fresh = paths.filter((p) => !existing.has(p));
                    return fresh.length ? [...prev, ...fresh] : prev;
                });
            }
        });
        return () => {
            unlisten.then((dispose) => dispose());
        };
    }, []);

    const browse = async () => {
        try {
            const picked = await open({ multiple: true });
            setFiles((prev) => {
                const existing = new Set(prev);
                const fresh = (Array.isArray(picked) ? picked : picked ? [picked] : []).filter(
                    (p) => !existing.has(p)
                );
                return fresh.length ? [...prev, ...fresh] : prev;
            });
        } catch (e) {
            setResults([{ inputPath: 'Dialog error', outputPaths: [], failure: { kind: 'processing', message: String(e) }, detail: '' }]);
        }
    };

    return (
        <div className="tool-scaffold">
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
