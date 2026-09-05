import { useState } from 'react';
import { ToolScaffold } from '../components/ToolScaffold';
import { invoke } from '@tauri-apps/api/core';
import type { PasswordRequest, ToolDefinition, ToolResult } from '../contracts';

export const PDFUnlockView = ({ utility }: { utility: ToolDefinition }) => {
    const [password, setPassword] = useState('');

    return (
        <ToolScaffold utility={utility} onRun={(paths) => invoke<ToolResult>('remove_password', { request: { paths, password, outputLocation: 'alongsideInput' } satisfies PasswordRequest })}>
            {({ files, run, loading }) => (
                <div className="space-y-6">
                    <p className="text-sm text-slate-600" aria-live="polite">
                        Supports PDF, DOC/DOCX, XLS/XLSX, and PPT/PPTX. Office files are processed locally.
                    </p>
                    <div className="flex flex-col space-y-2 max-w-sm">
                        <label className="text-sm font-medium text-slate-700">File Password</label>
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
                        Remove Password
                    </button>
                </div>
            )}
        </ToolScaffold>
    );
};
