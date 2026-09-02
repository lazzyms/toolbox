import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ToolScaffold } from '../components/ToolScaffold';
import type { ToolDefinition, ToolResult } from '../contracts';

export const ImageCompressView = ({ utility }: { utility: ToolDefinition }) => {
    const [quality, setQuality] = useState(80);

    return (
        <ToolScaffold utility={utility}>
            {({ files, setResults, loading, setLoading }) => (
                <div className="space-y-6">
                    <div className="flex flex-col space-y-2 max-w-sm">
                        <div className="flex justify-between">
                            <label className="text-sm font-medium text-slate-700">Quality</label>
                            <span className="text-sm font-bold text-blue-600">{quality}%</span>
                        </div>
                        <input
                            type="range"
                            min="1"
                            max="100"
                            value={quality}
                            onChange={(e) => setQuality(parseInt(e.target.value))}
                            className="w-full h-2 bg-slate-200 rounded-lg appearance-none cursor-pointer accent-blue-600"
                        />
                        <div className="flex justify-between text-[10px] text-slate-400">
                            <span>Smaller File</span>
                            <span>Higher Quality</span>
                        </div>
                    </div>

                    <button
                        disabled={loading || files.length === 0}
                        onClick={async () => {
                            setLoading(true);
                            try {
                                const res = await invoke<ToolResult>('compress_images', { paths: files, quality });
                                setResults(res);
                            } catch (e) {
                                alert(e);
                            } finally {
                                setLoading(false);
                            }
                        }}
                        className="bg-green-600 text-white px-6 py-2 rounded-lg font-medium hover:bg-green-700 disabled:opacity-50 transition-colors"
                    >
                        Compress Images
                    </button>
                </div>
            )}
        </ToolScaffold>
    );
};
