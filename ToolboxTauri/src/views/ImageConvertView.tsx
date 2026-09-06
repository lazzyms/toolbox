import { useState } from 'react';
import { ToolScaffold } from '../components/ToolScaffold';
import { invoke } from '@tauri-apps/api/core';
import type { ConvertImagesRequest, ToolDefinition, ToolResult } from '../contracts';

export const ImageConvertView = ({ utility }: { utility: ToolDefinition }) => {
    const [format, setFormat] = useState<ConvertImagesRequest['format']>('png');
    const formats = [
        { id: 'png', label: 'PNG', icon: '🖼️' },
        { id: 'jpg', label: 'JPEG', icon: '📷' },
        { id: 'webp', label: 'WebP', icon: '🌐' },
        { id: 'heic', label: 'HEIC', icon: '📱' },
    ] as const;

    return (
        <ToolScaffold utility={utility} onRun={(paths) => invoke<ToolResult>('convert_images', { request: { paths, format, outputLocation: 'alongsideInput' } satisfies ConvertImagesRequest })}>
            {({ files, run, loading }) => (
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
                        onClick={run}
                        className="bg-blue-600 text-white px-6 py-2 rounded-lg font-medium hover:bg-blue-700 disabled:opacity-50 transition-colors"
                    >
                        Convert Images
                    </button>
                </div>
            )}
        </ToolScaffold>
    );
};
