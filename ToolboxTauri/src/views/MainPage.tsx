import { useState } from 'react';
import { Utility, UtilityRegistry } from '../registry';
import { PDFUnlockView } from './PDFUnlockView';
import { PDFProtectView } from './PDFProtectView';
import { ImageCompressView } from './ImageCompressView';
import { ImageConvertView } from './ImageConvertView';
import { TablerIcon } from '../components/TablerIcon';

export const MainPage = () => {
    const [selectedTool, setSelectedTool] = useState<Utility | null>(null);

    return (
        <div className="flex h-screen bg-slate-50 text-slate-900 font-sans selection:bg-blue-100">
            {/* Sidebar */}
            <div className="w-72 bg-white border-r border-slate-200 p-6 flex flex-col">
                <div className="flex items-center space-x-3 mb-10 px-2">
                    <div className="w-8 h-8 bg-gradient-to-br from-amber-400 to-orange-500 rounded-lg flex items-center justify-center shadow-sm">
                        <TablerIcon name="briefcase" color="#172033" className="w-5 h-5" />
                    </div>
                    <h1 className="text-xl font-bold tracking-tight">Toolbox</h1>
                </div>

                <div className="flex-1 space-y-1">
                    {UtilityRegistry.map(tool => (
                        <button
                            key={tool.id}
                            onClick={() => setSelectedTool(tool)}
                            className={`w-full text-left px-3 py-2 rounded-xl transition-all duration-200 flex items-center group ${
                                selectedTool?.id === tool.id
                                ? 'bg-blue-50 text-blue-700 shadow-sm'
                                : 'text-slate-600 hover:bg-slate-100 hover:text-slate-900'
                            }`}
                        >
                            <TablerIcon name={tool.symbol} color={tool.tint} className="mr-3 w-5 h-5 group-hover:scale-110 transition-transform" />
                            <span className="text-sm font-medium">{tool.shortTitle}</span>
                        </button>
                    ))}
                </div>

                <div className="mt-auto p-4 bg-slate-100 rounded-2xl">
                    <p className="text-[11px] text-slate-400 text-center font-medium uppercase tracking-wider">Unified Native Engine</p>
                </div>

                <p className="mt-4 text-center text-xs text-slate-400">
                    Built with <span aria-hidden="true">❤️</span> by{' '}
                    <a
                        className="font-medium text-slate-500 underline decoration-slate-300 underline-offset-2 transition-colors hover:text-blue-600"
                        href="https://mauliksompura.co.in"
                        rel="noreferrer"
                        target="_blank"
                    >
                        lazzyms
                    </a>
                </p>
            </div>

            {/* Detail Area */}
            <div className="flex-1 p-12 overflow-auto bg-gradient-to-br from-slate-50 to-slate-100">
                {selectedTool ? (
                    <div className="max-w-4xl mx-auto h-full">
                        {selectedTool.id === 'pdf-unlock' && <PDFUnlockView utility={selectedTool} />}
                        {selectedTool.id === 'pdf-protect' && <PDFProtectView utility={selectedTool} />}
                        {selectedTool.id === 'image-compress' && <ImageCompressView utility={selectedTool} />}
                        {selectedTool.id === 'image-convert' && <ImageConvertView utility={selectedTool} />}
                    </div>
                ) : (
                    <div className="h-full flex flex-col items-center justify-center text-center space-y-4">
                        <div className="w-20 h-20 bg-slate-200 rounded-full flex items-center justify-center opacity-50">
                            <TablerIcon name="briefcase" color="#475569" className="w-10 h-10" />
                        </div>
                        <div>
                            <h3 className="text-xl font-semibold text-slate-700">Ready to process</h3>
                            <p className="text-slate-400">Select a utility from the sidebar to begin</p>
                        </div>
                    </div>
                )}
            </div>
        </div>
    );
};
