import { useState } from "react";
import { createRoot } from "react-dom/client";
import "../../src/design-system/primitives.css";
import "../../src/design-system/tokens.css";
import {
  Badge,
  Button,
  Card,
  ColorSwatches,
  EmptyState,
  IconButton,
  NumberField,
  Panel,
  Pill,
  Section,
  Select,
  SegmentedControl,
  Slider,
  StatusBar,
  TextField,
  Thumbnail,
  Toggle,
  Toolbar,
  ToolbarButton,
  Tooltip,
} from "../../src/design-system";

function DesignSystemPreview() {
  const [saved, setSaved] = useState(false);
  const [quality, setQuality] = useState(70);
  const [color, setColor] = useState("var(--color-action)");
  const [includeMetadata, setIncludeMetadata] = useState(false);
  const [orientation, setOrientation] = useState("portrait");
  const [name, setName] = useState("");
  const [copies, setCopies] = useState(1);
  const [format, setFormat] = useState("png");
  const [emptyStateAction, setEmptyStateAction] = useState(false);

  return (
    <main className="ds-preview">
      <h1>Design system preview</h1>
      <Panel title="Controls" description="Interactive primitive coverage">
        <div className="ds-preview-grid">
          <Button variant="primary" onClick={() => setSaved(true)}>
            Save changes
          </Button>
          <Button variant="danger">Delete item</Button>
          <Button disabled>Disabled action</Button>
          <IconButton aria-label="Close preview" onClick={() => setSaved(false)}>
            ×
          </IconButton>
          <IconButton aria-label="Disabled icon" disabled>
            ×
          </IconButton>
          <Slider label="Quality" value={quality} min={0} max={100} onChange={setQuality} />
          <Slider label="Disabled quality" value={40} min={0} max={100} disabled onChange={() => undefined} />
          <ColorSwatches
            label="Annotation color"
            value={color}
            options={[
              { label: "Action blue", value: "var(--color-action)" },
              { label: "Danger red", value: "var(--color-danger)" },
            ]}
            onChange={setColor}
          />
          <ColorSwatches
            label="Disabled colors"
            value="var(--color-action)"
            options={[{ label: "Action blue", value: "var(--color-action)" }]}
            disabled
            onChange={() => undefined}
          />
          <Toggle label="Keep metadata" checked={includeMetadata} onChange={setIncludeMetadata} />
          <Toggle label="Disabled metadata" checked={false} disabled onChange={() => undefined} />
          <SegmentedControl
            label="Orientation"
            value={orientation}
            options={[
              { label: "Portrait", value: "portrait" },
              { label: "Landscape", value: "landscape" },
            ]}
            onChange={setOrientation}
          />
          <SegmentedControl
            label="Disabled layout"
            value="grid"
            options={[
              { label: "Grid", value: "grid" },
              { label: "List", value: "list" },
            ]}
            disabled
            onChange={() => undefined}
          />
          <TextField label="Project name" value={name} onChange={(event) => setName(event.target.value)} />
          <TextField label="Disabled field" disabled />
          <NumberField label="Copies" value={copies} min={1} max={9} onChange={(event) => setCopies(Number(event.target.value))} />
          <NumberField label="Disabled copies" value={1} disabled onChange={() => undefined} />
          <Select
            label="Format"
            value={format}
            options={[
              { label: "PNG", value: "png" },
              { label: "WebP", value: "webp" },
            ]}
            onChange={(event) => setFormat(event.target.value)}
          />
          <Select label="Disabled format" value="png" options={[{ label: "PNG", value: "png" }]} disabled onChange={() => undefined} />
        </div>
        <div className="ds-preview-row">
          <Pill>Ready</Pill>
          <Badge tone="success">Saved</Badge>
        </div>
      </Panel>
      <Section title="Surfaces" description="Layout and feedback primitive coverage">
        <Card as="article" aria-label="Example card">Reusable card</Card>
        <Thumbnail src="data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=" alt="Example thumbnail" caption="Source image" />
        <Tooltip content="A local, non-destructive action">
          <Button variant="ghost">Help</Button>
        </Tooltip>
        <EmptyState
          title="Nothing open"
          description="Choose a local file to begin."
          action={<Button onClick={() => setEmptyStateAction(true)}>Choose file</Button>}
        />
        <Toolbar label="Edit controls">
          <ToolbarButton aria-label="Undo" onClick={() => setSaved(false)}>
            ↶
          </ToolbarButton>
          <ToolbarButton aria-label="Redo" disabled>
            ↷
          </ToolbarButton>
          <ToolbarButton aria-label="Copy" onClick={() => setSaved(false)}>
            ⧉
          </ToolbarButton>
        </Toolbar>
      </Section>
      <StatusBar>
        {saved ? "Saved" : "Draft"} · {quality}% · {format} · {copies} copy{copies === 1 ? "" : "ies"}
        {includeMetadata ? " · Metadata kept" : ""}
        {emptyStateAction ? " · File selected" : ""}
      </StatusBar>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(<DesignSystemPreview />);
