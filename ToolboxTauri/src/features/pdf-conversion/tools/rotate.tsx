import { invoke } from "@tauri-apps/api/core";
import type { ToolResult } from "../../../contracts";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import type { ConversionTool } from "../tool";

type Options = { degrees: number };

export const rotate: ConversionTool<Options> = {
  initialOptions: { degrees: 90 },
  inspectsPdf: true,
  outputPreview: () => "Output preview unavailable. Export saves a copy with the selected pages turned.",
  OptionsPanel: ({ options, setOptions }) => (
    <div className="workspace-field">
      <Label htmlFor="rotate-direction">Direction</Label>
      <NativeSelect id="rotate-direction" aria-label="Rotation direction" value={String(options.degrees)} onChange={(event) => setOptions({ degrees: Number(event.target.value) })}>
        <option value="90">Clockwise 90°</option>
        <option value="270">Counterclockwise 90°</option>
        <option value="180">180°</option>
      </NativeSelect>
    </div>
  ),
  request: (paths, outputLocation, { runId }, { degrees }, { document, selectedPages }) => {
    const somePages = document && selectedPages.length < document.pages.length;
    return invoke<ToolResult>("rotate_pdf", { runId,
      request: { paths, degrees, pages: somePages ? selectedPages : null, outputLocation },
    });
  },
};
