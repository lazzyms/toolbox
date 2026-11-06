import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import { parseEntryFacts, type EntryFacts } from "../components/intake";
import { ConversionPanel, conversionToolFor } from "../features/pdf-conversion";
import type { PdfDocument } from "../features/pdf-editor";
import type { AtomicToolId, ToolDefinition } from "../contracts";

const unknownFacts: EntryFacts = { pageCount: null, modifiedMs: null, sizeBytes: null };

export const PDFConversionWorkspaceView = ({ utility: activeUtility, initialPaths = [], onFilesChange, onSelectTool }: { utility: ToolDefinition; initialPaths?: readonly string[]; onFilesChange?: (paths: readonly string[]) => void; onSelectTool?: (id: AtomicToolId) => void }) => {
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
  const [listedPaths, setListedPaths] = useState<readonly string[]>(initialPaths);
  const [facts, setFacts] = useState<ReadonlyMap<string, EntryFacts>>(() => new Map());
  const describesFiles = activeUtility.id === "pdf-merge";
  const { exampleName } = tool;

  const listChanged = useCallback((paths: readonly string[]) => {
    setListedPaths(paths);
    onFilesChange?.(paths);
  }, [onFilesChange]);

  // Merge shows each file's page count, size and date. Files that can't be read simply show none.
  useEffect(() => {
    if (!describesFiles) return;
    const unknown = [...new Set(listedPaths)].filter((path) => !facts.has(path));
    if (unknown.length === 0) return;
    let current = true;
    // Every asked path is remembered, described or not, so a file that can't be read is asked about once.
    const remember = (described: ReadonlyMap<string, EntryFacts>) => {
      if (!current) return;
      setFacts((known) => new Map([...known, ...unknown.map((path) => [path, described.get(path) ?? unknownFacts] as const)]));
    };
    invoke("describe_intake_files", { paths: unknown })
      .then((value) => remember(parseEntryFacts(value)))
      .catch(() => remember(new Map()));
    return () => {
      current = false;
    };
  }, [describesFiles, listedPaths, facts]);

  return (
    <ToolScaffold
      key={initialPaths.join('\0') || 'no-initial-pdf'}
      variant="workspace"
      sessionKey="pdf-convert"
      utility={activeUtility}
      initialPaths={initialPaths}
      onFilesChange={listChanged}
      entryFacts={describesFiles ? (path) => facts.get(path) : undefined}
      listHint={describesFiles && listedPaths.length === 1 ? "Add at least one more PDF. You can add the same file again to repeat it." : undefined}
      exampleName={exampleName ? (path) => exampleName(path, activeUtility, options) : undefined}
      onRun={(paths, outputLocation, run) => tool.request(paths, outputLocation, run, options, { document, selectedPages, pageRange })}
    >
      {({ files, run, loading, destination, runLabel }) => (
        <ConversionPanel
          destination={destination}
          runLabel={runLabel}
          activeUtility={activeUtility}
          tool={tool}
          options={options}
          setOptions={setOptions}
          files={files}
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
          onSelectTool={onSelectTool}
        />
      )}
    </ToolScaffold>
  );
};
