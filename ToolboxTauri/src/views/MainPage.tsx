import { useState, type ComponentType } from 'react';
import { UtilityRegistry } from '../registry';
import type { ToolDefinition } from '../contracts';
import { PDFUnlockView } from './PDFUnlockView';
import { PDFProtectView } from './PDFProtectView';
import { ImageCompressView } from './ImageCompressView';
import { ImageConvertView } from './ImageConvertView';
import { TablerIcon } from '../components/TablerIcon';
import { PlannedToolView } from './PlannedToolView';
import { PDFCropView, PDFOrganizeView, PDFSignView } from './PDFPageToolView';
import { PDFSimpleToolView } from './PDFSimpleToolView';
import { PDFSelectionView } from './PDFSelectionView';
import { PDFPathsView } from './PDFPathsView';
import { PDFConversionView } from './PDFConversionView';
import { VisionView } from './VisionView';
import { ImageGeometryView } from './ImageGeometryView';
import { ImageEffectView } from './ImageEffectView';
import { ImageFormatView } from './ImageFormatView';
import { ImageMetadataView } from './ImageMetadataView';

const views = {
    'pdf-unlock': PDFUnlockView,
    'pdf-protect': PDFProtectView,
    'pdf-crop': PDFCropView,
    'pdf-sign': PDFSignView,
    'pdf-organize': PDFOrganizeView,
    'pdf-page-numbers': (props) => <PDFSimpleToolView {...props} mode="pageNumbers" />,
    'pdf-watermark': (props) => <PDFSimpleToolView {...props} mode="watermark" />,
    'pdf-compress': (props) => <PDFSimpleToolView {...props} mode="compress" />,
    'pdf-remove-pages': (props) => <PDFSelectionView {...props} mode="remove" />,
    'pdf-extract-pages': (props) => <PDFSelectionView {...props} mode="extract" />,
    'pdf-merge': (props) => <PDFPathsView {...props} mode="merge" />,
    'pdf-split': (props) => <PDFPathsView {...props} mode="split" />,
    'pdf-to-images': (props) => <PDFConversionView {...props} mode="to-images" />,
    'pdf-to-text': (props) => <PDFConversionView {...props} mode="to-text" />,
    'pdf-image-extract': (props) => <PDFConversionView {...props} mode="extract-images" />,
    'images-to-pdf': (props) => <PDFConversionView {...props} mode="images-to-pdf" />,
    'pdf-ocr': (props) => <VisionView {...props} mode="ocr" />,
    'image-blur-faces': (props) => <VisionView {...props} mode="faces" />,
    'image-remove-bg': (props) => <VisionView {...props} mode="background" />,
    'image-resize': (props) => <ImageGeometryView {...props} mode="resize" />,
    'image-rotate': (props) => <ImageGeometryView {...props} mode="rotate" />,
    'image-crop': (props) => <ImageGeometryView {...props} mode="crop" />,
    'image-watermark': (props) => <ImageEffectView {...props} mode="watermark" />,
    'image-tone': (props) => <ImageEffectView {...props} mode="tone" />,
    'image-icons': (props) => <ImageFormatView {...props} mode="icons" />,
    'gif-create': (props) => <ImageFormatView {...props} mode="gif-create" />,
    'gif-extract': (props) => <ImageFormatView {...props} mode="gif-extract" />,
    'tiff-pages': (props) => <ImageFormatView {...props} mode="tiff" />,
    'image-metadata': ImageMetadataView,
    'image-compress': ImageCompressView,
    'image-convert': ImageConvertView,
    planned: PlannedToolView,
} satisfies Record<ToolDefinition["view"], ComponentType<{ utility: ToolDefinition }>>;

export const MainPage = () => {
    const [selectedTool, setSelectedTool] = useState<ToolDefinition | null>(null);

    return (
        <div className="flex h-screen bg-slate-50 text-slate-900 font-sans selection:bg-blue-100">
            <a href="#tool-detail" className="sr-only focus:not-sr-only focus:absolute focus:z-10 focus:bg-white focus:p-2">Skip to tool</a>
            <aside aria-label="Toolbox navigation" className="w-72 bg-white border-r border-slate-200 p-6 flex flex-col">
                <div className="flex items-center space-x-3 mb-10 px-2">
                    <div className="w-8 h-8 bg-gradient-to-br from-amber-400 to-orange-500 rounded-lg flex items-center justify-center shadow-sm">
                        <TablerIcon name="briefcase" color="#172033" className="w-5 h-5" />
                    </div>
                    <h1 className="text-xl font-bold tracking-tight">Toolbox</h1>
                </div>

                <nav aria-label="Utilities" className="flex-1 space-y-1">
                    {UtilityRegistry.map(tool => (
                        <button
                            key={tool.id}
                            type="button"
                            onClick={() => setSelectedTool(tool)}
                            aria-current={selectedTool?.id === tool.id ? 'page' : undefined}
                            aria-label={`${tool.title}: ${tool.blurb}`}
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
                </nav>

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
            </aside>

            <main id="tool-detail" tabIndex={-1} aria-label="Tool detail" className="flex-1 p-12 overflow-auto bg-gradient-to-br from-slate-50 to-slate-100">
                {selectedTool ? (
                    <div className="max-w-4xl mx-auto h-full">
                        {(() => {
                            const View = views[selectedTool.view];
                            return <View utility={selectedTool} />;
                        })()}
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
                <p className="sr-only" role="status" aria-live="polite">
                    {selectedTool ? `${selectedTool.title} selected.` : "No tool selected."}
                </p>
            </main>
        </div>
    );
};
