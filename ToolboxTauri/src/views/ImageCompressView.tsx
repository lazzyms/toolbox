import { useState } from 'react';
import { ToolScaffold } from '../components/ToolScaffold';
import { invoke } from '@tauri-apps/api/core';
import type { CompressImagesRequest, ToolDefinition, ToolResult } from '../contracts';

export const ImageCompressView = ({ utility }: { utility: ToolDefinition }) => {
    const [quality, setQuality] = useState(80);
    const [lossless, setLossless] = useState(false);

    return (
        <ToolScaffold utility={utility} onRun={(paths) => invoke<ToolResult>('compress_images', { request: { paths, quality, lossless, outputLocation: 'alongsideInput' } satisfies CompressImagesRequest })}>
            {({ files, run, loading }) => (
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

                    <label className="flex items-center gap-2 text-sm text-slate-700">
                        <input aria-label="Lossless compression" type="checkbox" checked={lossless} onChange={(e) => setLossless(e.target.checked)} />
                        Lossless (preserve original bytes)
                    </label>

                    <button
                        disabled={loading || files.length === 0}
                        onClick={run}
                        className="bg-green-600 text-white px-6 py-2 rounded-lg font-medium hover:bg-green-700 disabled:opacity-50 transition-colors"
                    >
                        Compress Images
                    </button>
                </div>
            )}
        </ToolScaffold>
    );
};
