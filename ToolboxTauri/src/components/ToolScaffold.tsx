import { useState, useEffect } from 'react';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open } from '@tauri-apps/plugin-dialog';
import { Utility } from '../registry';

interface ToolScaffoldProps {
    utility: Utility;
    children: (props: {
        files: string[];
        setFiles: (files: string[]) => void;
        run: () => Promise<void>;
        results: any[];
        setResults: (results: any[]) => void;
        loading: boolean;
        setLoading: (loading: boolean) => void;
    }) => React.ReactNode;
}

export const ToolScaffold = ({ utility, children }: ToolScaffoldProps) => {
    const [files, setFiles] = useState<string[]>([]);
    const [results, setResults] = useState<any[]>([]);
    const [loading, setLoading] = useState(false);

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
        const picked = await open({ multiple: true });
        setFiles((prev) => {
            const existing = new Set(prev);
            const fresh = (Array.isArray(picked) ? picked : picked ? [picked] : []).filter(
                (p) => !existing.has(p)
            );
            return fresh.length ? [...prev, ...fresh] : prev;
        });
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
                            {files.map((f, i) => (
                                <div key={i} className="text-xs text-slate-500 truncate p-1 border-b last:border-0">
                                    {f.split('/').pop()}
                                </div>
                            ))}
                        </div>
                    </div>
                )}
            </div>

            {children({
                files,
                setFiles,
                run: async () => {},
                results,
                setResults,
                loading,
                setLoading
            })}

            {loading && (
                <div className="fixed bottom-8 right-8 bg-white shadow-xl border rounded-full px-6 py-3 flex items-center space-x-4">
                    <div className="w-4 h-4 border-2 border-blue-600 border-t-transparent rounded-full animate-spin"></div>
                    <span className="text-sm font-medium text-slate-700">Processing batch...</span>
                </div>
            )}
        </div>
    );
};