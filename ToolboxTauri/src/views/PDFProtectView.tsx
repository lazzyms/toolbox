import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ToolScaffold } from '../components/ToolScaffold';
import type { ToolDefinition, ToolResult } from '../contracts';

export const PDFProtectView = ({ utility }: { utility: ToolDefinition }) => {
    const [password, setPassword] = useState('');

    return (
        <ToolScaffold utility={utility}>
            {({ files, setResults, loading, setLoading }) => (
                <div className="space-y-6">
                    <div className="flex flex-col space-y-2 max-w-sm">
                        <label className="text-sm font-medium text-slate-700">Encryption Password</label>
                        <input
                            type="password"
                            value={password}
                            onChange={(e) => setPassword(e.target.value)}
                            className="px-4 py-2 border rounded-lg focus:ring-2 focus:ring-blue-500 outline-none"
                            placeholder="Set password..."
                        />
                    </div>

                    <button
                        disabled={loading || files.length === 0 || !password}
                        onClick={async () => {
                            setLoading(true);
                            try {
                                const res = await invoke<ToolResult>('protect_pdf', { paths: files, password });
                                setResults(res);
                            } catch (e) {
                                alert(e);
                            } finally {
                                setLoading(false);
                            }
                        }}
                        className="bg-red-600 text-white px-6 py-2 rounded-lg font-medium hover:bg-red-700 disabled:opacity-50 transition-colors"
                    >
                        Protect PDF
                    </button>
                </div>
            )}
        </ToolScaffold>
    );
};
