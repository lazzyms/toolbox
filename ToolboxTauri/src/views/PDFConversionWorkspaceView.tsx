import { useCallback, useState } from "react";
import { ToolScaffold } from "../components/ToolScaffold";
import { ConversionPanel, conversionToolFor } from "../features/pdf-conversion";
import type { PdfDocument } from "../features/pdf-editor";
import type { AtomicToolId, ToolDefinition } from "../contracts";

export const PDFConversionWorkspaceView = ({ utility: activeUtility, initialPaths = [], onFilesChange }: { utility: ToolDefinition; initialPaths?: readonly string[]; onFilesChange?: (paths: readonly string[]) => void }) => {
  const tool = conversionToolFor(activeUtility.id);
  const [optionsByTool, setOptionsByTool] = useState<Partial<Record<AtomicToolId, unknown>>>({});
  const options = optionsByTool[activeUtility.id] ?? tool.initialOptions;
  const setOptions = useCallback(
    (next: unknown) => setOptionsByTool((current) => ({ ...current, [activeUtility.id]: next })),
    [activeUtility.id],
  );
  const [pageRange, setPageRange] = useState("");
  const [document, setDocument] = useState<PdfDocument | null>(null);
  const [selectedPages, setSelectedPages] = useState<number[]>([]);
  const [inspectError, setInspectError] = useState("");
  const { exampleName } = tool;

  return (
    <ToolScaffold
      key={initialPaths.join('\0') || 'no-initial-pdf'}
      variant="workspace"
      sessionKey="pdf-convert"
      utility={activeUtility}
      initialPaths={initialPaths}
      onFilesChange={onFilesChange}
      exampleName={exampleName ? (path) => exampleName(path, activeUtility, options) : undefined}
      onRun={(paths, outputLocation, run) => tool.request(paths, outputLocation, run, options, { document, selectedPages, pageRange })}
    >
      {({ files, selectedFileIndex, run, loading, destination, runLabel }) => (
        <ConversionPanel
          destination={destination}
          runLabel={runLabel}
          activeUtility={activeUtility}
          tool={tool}
          options={options}
          setOptions={setOptions}
          files={files}
          selectedFileIndex={selectedFileIndex}
          run={run}
          loading={loading}
          pageRange={pageRange}
          setPageRange={setPageRange}
          document={document}
          setDocument={setDocument}
          selectedPages={selectedPages}
          setSelectedPages={setSelectedPages}
          inspectError={inspectError}
          setInspectError={setInspectError}
        />
      )}
    </ToolScaffold>
  );
};
