import { useState, useEffect } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open } from '@tauri-apps/plugin-dialog';
import type { ToolDefinition, JobOutcome, Progress } from '../contracts';
import { ResultList } from './ResultList';

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
            setResults([{ inputPath: 'Tool error', outputPaths: [], failure: String(error), detail: '' }]);
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
            setResults([{ inputPath: 'Dialog error', outputPaths: [], failure: String(e), detail: '' }]);
        }
    };

    return (
        <div className="flex flex-col h-full">
            <div className="mb-6">
                <h2 className="text-3xl font-bold text-slate-900">{utility.title}</h2>
                <p className="text-slate-500">{utility.blurb}</p>
            </div>

            <div
                onClick={browse}
                className="flex-1 border-2 border-dashed border-slate-300 rounded-2xl p-8 flex flex-col items-center justify-center bg-slate-50 hover:border-blue-400 transition-colors cursor-pointer mb-6"
            >
                <div className="text-center pointer-events-none">
                    <div className="text-4xl mb-4">📁</div>
                    <p className="text-slate-600 font-medium">Drag & Drop files here</p>
                    <p className="text-slate-400 text-sm">or click to browse</p>
                </div>

                {files.length > 0 && (
                    <div className="mt-6 w-full max-w-md pointer-events-auto">
                        <div className="flex justify-between items-center mb-2">
                            <span className="text-sm font-semibold text-slate-700">{files.length} files selected</span>
                            <button
                                onClick={(e) => {
                                    e.stopPropagation();
                                    setFiles([]);
                                    setResults([]);
                                }}
                                className="text-xs text-red-500 hover:underline"
                            >
                                Clear all
                            </button>
                        </div>
                        <div className="max-h-40 overflow-y-auto border rounded-lg bg-white p-2 space-y-1">
                            {files.map((f) => (
                                    <div key={f} className="text-xs text-slate-500 truncate p-1 border-b last:border-0">
                                    {f.split('/').pop()}
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
