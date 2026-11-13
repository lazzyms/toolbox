import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import type { TiffCompression } from "../../contracts";
import { exampleOutputName } from "../../components/RunDestination";
import { TiffPageInspector, canCombineTiffPages, initialTiffPagesState, tiffPagesInOrder, type TiffMode, type TiffPagesState } from "./TiffPageInspector";
import type { MediaPanelProps, MediaTool } from "./types";

const compressions: { value: TiffCompression; name: string }[] = [
  { value: "lzw", name: "LZW, lossless (default)" },
  { value: "deflate", name: "Deflate, lossless" },
  { value: "none", name: "None, largest files" },
];

const TiffPagesPanel = ({ utility, options, setOptions, files, fileKeys, removeFile, browse, loading }: MediaPanelProps<TiffPagesState>) => (
  <>
    <ToggleGroup
      type="single"
      variant="outline"
      size="sm"
      aria-label="TIFF Pages mode"
      value={options.mode}
      onValueChange={(value) => {
        if (value) setOptions((current) => ({ ...current, mode: value as TiffMode }));
      }}
    >
      <ToggleGroupItem value="split">Split pages</ToggleGroupItem>
      <ToggleGroupItem value="combine">Combine into one TIFF</ToggleGroupItem>
    </ToggleGroup>
    {options.mode === "split" ? (
      <p className="workspace-panel-copy">
        Each page of every TIFF becomes its own TIFF. Page numbers are padded so they sort. An existing name gets -1, -2 and so on.
        A TIFF with only 1 page is skipped.
      </p>
    ) : (
      <TiffPageInspector
        files={files}
        fileKeys={fileKeys}
        removeFile={removeFile}
        browse={browse}
        supportsPreview={utility.capability.supportsPreview}
        disabled={loading}
        state={options}
        setState={setOptions}
      />
    )}
    <div className="workspace-field">
      <Label htmlFor="tiff-pages-compression">Compression</Label>
      <NativeSelect
        id="tiff-pages-compression"
        aria-describedby="tiff-pages-compression-hint"
        value={options.compression}
        disabled={loading}
        onChange={(event) => {
          const compression = event.target.value as TiffCompression;
          setOptions((current) => ({ ...current, compression }));
        }}
      >
        {compressions.map(({ value, name }) => <option key={value} value={value}>{name}</option>)}
      </NativeSelect>
      <p id="tiff-pages-compression-hint" className="workspace-panel-copy">
        The source compression isn't kept. Pages are saved at 8 bits per channel; 16-bit pages stay 16-bit.
      </p>
    </div>
  </>
);

const count = (value: number, one: string, many: string) => `${value} ${value === 1 ? one : many}`;

export const tiffPagesTool: MediaTool<TiffPagesState> = {
  initialOptions: initialTiffPagesState,
  Panel: TiffPagesPanel,
  // Split runs each file on its own, so it never waits on the page grid. Combine waits until the
  // pages of the current files have been read, and needs at least one page.
  canRun: (options, files, keys) => options.mode === "split" || canCombineTiffPages(options, files, keys),
  runLabel: (options, files, keys) => options.mode === "split"
    ? `Split ${count(files.length, "TIFF", "TIFFs")}`
    : `Combine ${count(tiffPagesInOrder(options, files, keys).length, "page", "pages")}…`,
  // The inspector orders pages, so the shared file list does not.
  showFileOrdering: false,
  // The registry describes Combine. Split is a per-file batch: no save step, files fail alone, and failed
  // files can be retried.
  capability: ({ mode }, registered) =>
    mode === "split" ? { ...registered, inputCardinality: "multiple", manyToOne: false } : registered,
  exampleName: ({ mode }, path) => exampleOutputName(path, "process_tiff_pages", mode === "split" ? "-page-01" : "-combined", "tif"),
  // Combine sends each page that goes into the TIFF, in the order shown, as its file and its index in that file.
  buildRequest: (options, { paths, keys, outputLocation, outputPath }) => ({
    command: "process_tiff_pages",
    request: options.mode === "split"
      ? { mode: options.mode, paths, outputLocation, compression: options.compression }
      : {
          mode: options.mode,
          paths,
          pages: tiffPagesInOrder(options, paths, keys).map(({ path, page }) => ({ path, page })),
          outputLocation,
          outputPath,
          compression: options.compression,
        },
  }),
};
