import { useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ToolScaffold } from "../components/ToolScaffold";
import type { OutputLocation, ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";
import { initialMediaOptions, isMediaToolId, mediaTools, type MediaOptions, type MediaToolId } from "../features/media";

type OptionSetters = { [Id in MediaToolId]: (update: (current: MediaOptions[Id]) => MediaOptions[Id]) => void };

export const MediaWorkspaceView = ({ utility: activeUtility, initialPaths, onFilesChange }: { utility: ToolDefinition; initialPaths?: readonly string[]; onFilesChange?: (paths: readonly string[]) => void }) => {
  // Every tool's options live here, so they survive switching tools on the rail.
  const [options, setOptions] = useState(initialMediaOptions);
  const setters = useMemo<OptionSetters>(() => {
    const setterFor = <Id extends MediaToolId>(id: Id) => (update: (current: MediaOptions[Id]) => MediaOptions[Id]) =>
      setOptions((all) => ({ ...all, [id]: update(all[id]) }));
    return {
      "icon-set": setterFor("icon-set"),
      "gif-create": setterFor("gif-create"),
      "gif-extract": setterFor("gif-extract"),
      "tiff-pages": setterFor("tiff-pages"),
      "image-metadata": setterFor("image-metadata"),
    };
  }, []);

  // Only Media tools reach this view (the vision tools in the Media rail are unavailable). Any other id runs as
  // image metadata, which the old fallback branch also did.
  const toolId: MediaToolId = isMediaToolId(activeUtility.id) ? activeUtility.id : "image-metadata";
  const tool = mediaTools[toolId];

  const buildRequest = <Id extends MediaToolId>(id: Id, paths: string[], outputLocation: OutputLocation, outputPath: string | null) =>
    mediaTools[id].buildRequest(options[id], { paths, outputLocation, outputPath });
  const canRun = <Id extends MediaToolId>(id: Id, files: string[]) =>
    mediaTools[id].canRun?.(options[id], files) ?? true;
  const runLabelFor = <Id extends MediaToolId>(id: Id, files: string[]) => {
    const label = mediaTools[id].runLabel;
    return typeof label === "function" ? label(options[id], files) : label;
  };
  const exampleNameFor = <Id extends MediaToolId>(id: Id) => {
    const exampleName = mediaTools[id].exampleName;
    return exampleName && ((path: string) => exampleName(options[id], path));
  };
  // A tool's options can change how the shared run treats its inputs, such as TIFF Split versus Combine.
  const capabilityFor = <Id extends MediaToolId>(id: Id) =>
    mediaTools[id].capability?.(options[id], activeUtility.capability) ?? activeUtility.capability;
  const capability = capabilityFor(toolId);
  const utility = useMemo(
    () => (capability === activeUtility.capability ? activeUtility : { ...activeUtility, capability }),
    [activeUtility, capability.inputCardinality, capability.manyToOne],
  );
  const renderPanel = <Id extends MediaToolId>(id: Id, scaffold: { files: string[]; outputLocation: OutputLocation; loading: boolean; selectedFileIndex: number; selectFile: (index: number) => void }): ReactNode => {
    const Panel = mediaTools[id].Panel;
    return <Panel key={id} utility={activeUtility} options={options[id]} setOptions={setters[id]} {...scaffold} />;
  };

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="media-tools"
      utility={utility}
      exampleName={exampleNameFor(toolId)}
      initialPaths={initialPaths}
      onFilesChange={onFilesChange}
      showFileOrdering={tool.showFileOrdering ?? true}
      outputGroup={tool.outputGroup}
      resultsHint={tool.resultsHint}
      onRun={(paths, outputLocation, { outputPath, runId }) => {
        const { command, request } = buildRequest(toolId, paths, outputLocation, outputPath);
        return invoke<ToolResult>(command, { runId, request });
      }}
    >
      {({ files, outputLocation, run, loading, selectedFileIndex, selectFile, destination, runLabel }) => (
        <Card className="workspace-control-panel py-0">
          <CardContent className="workspace-control-panel-content grid gap-4 p-4">
          <div>
            <h2 className="workspace-active-command">{activeUtility.title}</h2>
            <p className="workspace-panel-label">Specialized media outcome</p>
            <p className="workspace-panel-copy">Keep the same selected files while choosing a sequence, icon, or explicit metadata result.</p>
          </div>

          {renderPanel(toolId, { files, outputLocation, loading, selectedFileIndex, selectFile })}

          {destination}
          <Button variant="default" disabled={loading || files.length === 0 || !canRun(toolId, files)} onClick={run} className="workspace-primary-action" {...shortcutTarget("run")}>
            {runLabel ?? runLabelFor(toolId, files) ?? activeUtility.shortTitle}
            <ShortcutHint id="run" />
          </Button>
          </CardContent>
        </Card>
      )}
    </ToolScaffold>
  );
};
