import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CircleAlert } from "lucide-react";
import { ToolScaffold, type RunHandler } from "../components/ToolScaffold";
import type { AtomicToolId, ToolDefinition, ToolResult } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { ShortcutHint } from "../components/ShortcutHint";
import { shortcutTarget } from "../shortcuts";

type Mode = "crop" | "resize";
type Unit = "mm" | "in" | "pt";
type Side = "top" | "right" | "bottom" | "left";
type Margins = Record<Side, number>;

const sides: ReadonlyArray<{ key: Side; label: string }> = [
  { key: "top", label: "Top" },
  { key: "right", label: "Right" },
  { key: "bottom", label: "Bottom" },
  { key: "left", label: "Left" },
];

/** PDF points per unit. Lengths are converted once, when the request is built. */
const pointsPerUnit: Record<Unit, number> = { mm: 72 / 25.4, in: 72, pt: 1 };

const ERROR_ID = "mar-err";

type FieldCheck = { value: number } | { error: string };

const checkField = (label: string, text: string): FieldCheck => {
  const trimmed = text.trim();
  if (!trimmed) return { error: `${label} is empty; enter 0 for no margin.` };
  const value = Number(trimmed);
  if (!Number.isFinite(value)) return { error: `${label} must be a number.` };
  if (value < 0) return { error: `${label} can't be less than 0.` };
  return { value };
};

type MarginCheck = { margins: Margins; errors: [] } | { margins: null; errors: Array<{ side: Side | "all"; message: string }> };

const checkMargins = (same: boolean, all: string, fields: Record<Side, string>, unit: Unit): MarginCheck => {
  const scale = pointsPerUnit[unit];
  if (same) {
    const check = checkField("All sides", all);
    if ("error" in check) return { margins: null, errors: [{ side: "all", message: check.error }] };
    const points = check.value * scale;
    return { margins: { top: points, right: points, bottom: points, left: points }, errors: [] };
  }
  const errors: Array<{ side: Side; message: string }> = [];
  const margins = {} as Margins;
  for (const { key, label } of sides) {
    const check = checkField(label, fields[key]);
    if ("error" in check) errors.push({ side: key, message: check.error });
    else margins[key] = check.value * scale;
  }
  return errors.length ? { margins: null, errors } : { margins, errors: [] };
};

const cropWith = (margins: Margins): RunHandler => (paths, outputLocation, { runId }) =>
  invoke<ToolResult>("crop_pdf", {
    runId,
    request: { paths, spec: { kind: "margins", ...margins }, scope: "all", outputLocation },
  });

const pdfCount = (count: number) => `${count} ${count === 1 ? "PDF" : "PDFs"}`;

const UnitInput = ({ id, label, unit, value, invalid, onChange }: { id: string; label: string; unit: Unit; value: string; invalid: boolean; onChange: (value: string) => void }) => (
  <div className="workspace-field">
    <Label htmlFor={id}>{label}</Label>
    <div className="workspace-unit-input">
      <Input
        id={id}
        inputMode="decimal"
        value={value}
        aria-invalid={invalid || undefined}
        aria-describedby={invalid ? ERROR_ID : undefined}
        onChange={(event) => onChange(event.target.value)}
      />
      <span aria-hidden="true">{unit}</span>
    </div>
  </div>
);

export const CropResizeView = ({ utility, initialPaths, onFilesChange, onSelectTool }: {
  utility: ToolDefinition;
  initialPaths?: readonly string[];
  onFilesChange?: (paths: readonly string[]) => void;
  onSelectTool?: (id: AtomicToolId) => void;
}) => {
  const [mode, setMode] = useState<Mode>("crop");
  const [unit, setUnit] = useState<Unit>("mm");
  const [same, setSame] = useState(false);
  const [all, setAll] = useState("12");
  const [fields, setFields] = useState<Record<Side, string>>({ top: "15", right: "30", bottom: "15", left: "30" });
  const check = checkMargins(same, all, fields, unit);
  const invalidSides = new Set(check.errors.map((error) => error.side));

  return (
    <ToolScaffold
      variant="workspace"
      sessionKey="pdf-crop"
      utility={utility}
      initialPaths={initialPaths}
      onFilesChange={onFilesChange}
      onRun={check.margins ? cropWith(check.margins) : async () => { throw new Error("Fix the margins to crop."); }}
    >
      {({ files, run, loading, destination }) => {
        const runLabel = mode === "crop"
          ? files.length === 0 ? "Crop PDFs" : `Crop ${pdfCount(files.length)}`
          : files.length === 0 ? "Resize PDFs" : `Resize ${pdfCount(files.length)}`;
        const reason = mode === "resize"
          ? "Resize isn't available yet."
          : files.length === 0
            ? "Add PDFs to crop."
            : !check.margins ? "Fix the margins to crop." : null;
        return (
          <Card className="workspace-control-panel py-0">
            <CardContent className="workspace-control-panel-content grid gap-4 p-4">
              <div>
                <h2 className="workspace-active-command">{utility.title}</h2>
                <ToggleGroup
                  type="single"
                  variant="outline"
                  size="sm"
                  aria-label="Mode"
                  value={mode}
                  onValueChange={(value) => {
                    if (value) setMode(value as Mode);
                  }}
                >
                  <ToggleGroupItem value="crop">Crop pages</ToggleGroupItem>
                  <ToggleGroupItem value="resize">Resize pages</ToggleGroupItem>
                </ToggleGroup>
                <p className="workspace-panel-copy">
                  {mode === "crop" ? "Trim the edges of every page." : "Change the paper size of every page."}
                </p>
                <p className="workspace-note">Accepts .pdf</p>
              </div>
              {mode === "crop" && (
                <section className="grid gap-3" aria-label="Margins">
                  <div className="workspace-field-heading">
                    <Label className="workspace-check" htmlFor="crop-same-margins">
                      <Checkbox id="crop-same-margins" checked={same} onCheckedChange={(checked) => setSame(checked === true)} />
                      <span>Same on all sides</span>
                    </Label>
                    <div className="workspace-check">
                      <Label htmlFor="crop-margin-unit">Unit</Label>
                      <NativeSelect id="crop-margin-unit" size="sm" value={unit} onChange={(event) => setUnit(event.target.value as Unit)}>
                        <option value="mm">mm</option>
                        <option value="in">in</option>
                        <option value="pt">pt</option>
                      </NativeSelect>
                    </div>
                  </div>
                  {same ? (
                    <div className="workspace-field-grid">
                      <UnitInput id="crop-margin-all" label="All sides" unit={unit} value={all} invalid={invalidSides.has("all")} onChange={setAll} />
                    </div>
                  ) : (
                    <div className="workspace-field-grid">
                      {sides.map(({ key, label }) => (
                        <UnitInput
                          key={key}
                          id={`crop-margin-${key}`}
                          label={label}
                          unit={unit}
                          value={fields[key]}
                          invalid={invalidSides.has(key)}
                          onChange={(value) => setFields((current) => ({ ...current, [key]: value }))}
                        />
                      ))}
                    </div>
                  )}
                  {check.errors.length > 0 && (
                    <p id={ERROR_ID} role="alert" className="workspace-field-error">
                      <CircleAlert aria-hidden="true" />
                      <span>{check.errors.map((error) => error.message).join(" ")}</span>
                    </p>
                  )}
                  <p className="workspace-note">Every page loses the same lengths. A page left with no area fails its PDF.</p>
                  <div className="workspace-note grid justify-items-start gap-1" data-testid="crop-hidden-note">
                    <span>Crop hides the rest of the page. It stays in the file and can be shown again.</span>
                    <Button variant="link" size="sm" className="h-auto p-0" onClick={() => onSelectTool?.("pdf-redact")}>
                      Remove content with Redact PDF
                    </Button>
                  </div>
                </section>
              )}
              {destination}
              {mode === "crop" && <p className="workspace-note">Names end in -cropped. A taken name gets -1, -2.</p>}
              {reason && <p className={`workspace-note ${reason === "Fix the margins to crop." ? "workspace-note-error" : ""}`} data-testid="crop-run-reason">{reason}</p>}
              <Button
                variant="default"
                aria-label={runLabel}
                disabled={loading || reason !== null}
                onClick={run}
                className="workspace-primary-action"
                {...shortcutTarget("run")}
              >
                {runLabel}
                <ShortcutHint id="run" />
              </Button>
            </CardContent>
          </Card>
        );
      }}
    </ToolScaffold>
  );
};
