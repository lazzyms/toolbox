import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ToolScaffold } from '../components/ToolScaffold';
import { Utility } from '../registry';

export const ImageConvertView = ({ utility }: { utility: Utility }) => {
    const [format, setFormat] = useState('png');
    const formats = [
        { id: 'png', label: 'PNG', icon: '🖼️' },
        { id: 'jpg', label: 'JPEG', icon: '📷' },
        { id: 'webp', label: 'WebP', icon: '🌐' },
        { id: 'heic', label: 'HEIC', icon: '📱' },
    ];

    return (
        <ToolScaffold utility={utility}>
            {({ files, results, setResults, loading, setLoading }) => (
                <div className="space-y-6">
                    <div className="flex flex-col space-y-3 max-w-sm">
                        <label className="text-sm font-medium text-slate-700">Target Format</label>
                        <div className="grid grid-cols-4 gap-3">
                            {formats.map(f => (
                                <button
                                    key={f.id}
                                    onClick={() => setFormat(f.id)}
                                    className={`p-3 rounded-xl border transition-all flex flex-col items-center justify-center space-y-2 ${
                                        format === f.id
                                        ? 'bg-blue-50 border-blue-500 text-blue-700 ring-2 ring-blue-500/20'
                                        : 'bg-white border-slate-200 text-slate-600 hover:border-slate-300'
                                    }`}
                                >
                                    <span className="text-xl">{f.icon}</span>
                                    <span className="text-xs font-bold">{f.label}</span>
                                </button>
                            ))}
                        </div>
                    </div>

                    <button
                        disabled={loading || files.length === 0}
                        onClick={async () => {
                            setLoading(true);
                            try {
                                const res = await invoke('convert_images', { paths: files, format });
                                setResults(res as any[]);
                            } catch (e) {
                                alert(e);
                            } finally {
                                setLoading(false);
                            }
                        }}
                        className="bg-blue-600 text-white px-6 py-2 rounded-lg font-medium hover:bg-blue-700 disabled:opacity-50 transition-colors"
                    >
                        Convert Images
                    </button>

                    <div className="space-y-2">
                        {results.map((res, i) => (
                            <div key={i} className="p-3 bg-white border rounded-lg flex justify-between items-center">
                                <span className="text-sm truncate max-w-xs">{res.input_path.split('/').pop()}</span>
                                <span className={`text-xs font-medium px-2 py-1 rounded ${res.failure ? 'bg-red-100 text-red-600' : 'bg-green-100 text-green-600'}`}>
                                    {res.failure || res.detail}
                                </span>
                            </div>
                        ))}
                    </div>
                </div>
            )}
        </ToolScaffold>
    );
};
