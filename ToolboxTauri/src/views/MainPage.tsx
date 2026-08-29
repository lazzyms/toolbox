import React, { useState } from 'react';
import { invoke } from '@tauri-apps/api/tauri';
import { UtilityRegistry, Utility } from '../registry';

export const MainPage = () => {
    const [selectedTool, setSelectedTool] = useState<Utility | null>(null);
    const [files, setFiles] = useState<string[]>([]);
    const [results, setResults] = useState<any[]>([]);
    const [loading, setLoading] = useState(false);

    const handleRun = async () => {
        if (!selectedTool) return;
        setLoading(true);
        try {
            let outcomes;
            switch (selectedTool.id) {
                case 'pdf-unlock':
                    outcomes = await invoke('unlock_pdf', { paths: files, password: 'user_password' });
                    break;
                case 'pdf-protect':
                    outcomes = await invoke('protect_pdf', { paths: files, password: 'user_password' });
                    break;
                case 'image-compress':
                    outcomes = await invoke('compress_images', { paths: files, quality: 80 });
                    break;
                case 'image-convert':
                    outcomes = await invoke('convert_images', { paths: files, format: 'png' });
                    break;
            }
            setResults(outcomes as any[]);
        } catch (e) {
            console.error(e);
        } finally {
            setLoading(false);
        }
    };

    return (
        <div className="flex h-screen bg-slate-50">
            {/* Sidebar */}
            <div className="w-64 bg-white border-r border-slate-200 p-4 flex flex-col">
                <h1 className="text-xl font-bold mb-6">Toolbox</h1>
                <div className="flex-1">
                    {UtilityRegistry.map(tool => (
                        <button
                            key={tool.id}
                            onClick={() => setSelectedTool(tool)}
                            className={`w-full text-left p-2 rounded mb-1 flex items-center ${selectedTool?.id === tool.id ? 'bg-blue-100 text-blue-700' : 'hover:bg-slate-100'}`}
                        >
                            <span className="mr-3" style={{ color: tool.tint }}>{tool.symbol}</span>
                            {tool.shortTitle}
                        </button>
                    ))}
                </div>
            </div>

            {/* Detail Area */}
            <div className="flex-1 p-8 overflow-auto">
                {selectedTool ? (
                    <div className="max-w-3xl mx-auto">
                        <h2 className="text-3xl font-bold mb-2">{selectedTool.title}</h2>
                        <p className="text-slate-500 mb-8">{selectedTool.blurb}</p>

                        <div className="bg-white border-2 border-dashed border-slate-300 rounded-xl p-12 text-center mb-8 cursor-pointer hover:border-blue-400 transition-colors">
                            <p>Drag and drop files here or click to browse</p>
                            {files.length > 0 && <p className="mt-2 font-medium">{files.length} files selected</p>}
                        </div>

                        <button
                            onClick={handleRun}
                            disabled={loading || files.length === 0}
                            className="bg-blue-600 text-white px-6 py-2 rounded-lg font-medium hover:bg-blue-700 disabled:opacity-50"
                        >
                            {loading ? 'Processing...' : 'Run Tool'}
                        </button>

                        <div className="mt-8 space-y-2">
                            {results.map((res, i) => (
                                <div key={i} className="p-3 bg-white border rounded-lg flex justify-between">
                                    <span>{res.input_path}</span>
                                    <span className={res.failure ? 'text-red-500' : 'text-green-500'}>
                                        {res.failure || res.detail}
                                    </span>
                                </div>
                            ))}
                        </div>
                    </div>
                ) : (
                    <div className="h-full flex items-center justify-center text-slate-400">
                        Select a tool from the sidebar to get started
                    </div>
                )}
            </div>
        </div>
    );
};
