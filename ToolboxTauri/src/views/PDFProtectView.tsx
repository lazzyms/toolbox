import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ToolScaffold } from '../components/ToolScaffold';
import { Utility } from '../registry';

export const PDFProtectView = ({ utility }: { utility: Utility }) => {
    const [password, setPassword] = useState('');

    return (
        <ToolScaffold utility={utility}>
            {({ files, results, setResults, loading, setLoading }) => (
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
                                const res = await invoke('protect_pdf', { paths: files, password });
                                setResults(res as any[]);
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
