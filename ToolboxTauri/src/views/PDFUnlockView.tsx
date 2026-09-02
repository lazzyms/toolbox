import { useState } from 'react';
import { ToolScaffold } from '../components/ToolScaffold';
import { invoke } from '@tauri-apps/api/core';
import type { PDFRequest, ToolDefinition, ToolResult } from '../contracts';

export const PDFUnlockView = ({ utility }: { utility: ToolDefinition }) => {
    const [password, setPassword] = useState('');

    return (
        <ToolScaffold utility={utility} onRun={(paths) => invoke<ToolResult>('unlock_pdf', { request: { paths, password, outputLocation: 'alongsideInput' } satisfies PDFRequest })}>
            {({ files, run, loading }) => (
                <div className="space-y-6">
                    <div className="flex flex-col space-y-2 max-w-sm">
                        <label className="text-sm font-medium text-slate-700">PDF Password</label>
                        <input
                            type="password"
                            value={password}
                            onChange={(e) => setPassword(e.target.value)}
                            className="px-4 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 outline-none"
                            placeholder="Enter password..."
                        />
                    </div>

                    <button
                        disabled={loading || files.length === 0 || !password}
                        onClick={run}
                        className="bg-blue-600 text-white px-6 py-2 rounded-lg font-medium hover:bg-blue-700 disabled:opacity-50 transition-colors"
                    >
                        Unlock PDF
                    </button>
                </div>
            )}
        </ToolScaffold>
    );
};
