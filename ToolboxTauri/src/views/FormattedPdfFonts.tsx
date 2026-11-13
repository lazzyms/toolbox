import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Info } from "lucide-react";
import type { FontStyle, FontUse, JobOutcome } from "../contracts";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "@/components/ui/select";
import { isWindows } from "../shortcuts";

/** The document's text font, sent with the document; headings scale from its size. */
export type DocumentFont = { family: string; size: number };
export const defaultFont: DocumentFont = { family: "Liberation Serif", size: 11 };
/** The base sizes the board offers, in points. */
export const BASE_SIZES = { min: 8, max: 24 } as const;

export const fontSummary = (font: DocumentFont) => `${font.family}, ${font.size} pt`;

/** What list_formatted_pdf_fonts returns: the bundled families, then the ones installed on this computer. */
type FontChoices = { bundled: string[]; system: string[] };

const isChoices = (value: unknown): value is FontChoices => {
  const { bundled, system } = (value ?? {}) as Partial<FontChoices>;
  return Array.isArray(bundled) && Array.isArray(system) && [...bundled, ...system].every((name) => typeof name === "string");
};

/** The computer a font from this one may not match on. */
const otherComputer = () => (isWindows() ? "a Mac" : "Windows");

/** The families to offer: the bundled ones as listed, then installed ones. Until the list arrives, the chosen family. */
const useFontChoices = () => {
  const [choices, setChoices] = useState<FontChoices>({ bundled: [defaultFont.family], system: [] });
  useEffect(() => {
    let live = true;
    invoke("list_formatted_pdf_fonts")
      .then((result) => { if (live && isChoices(result)) setChoices(result); })
      .catch(() => { /* Without the list, the bundled default stays the only choice. */ });
    return () => { live = false; };
  }, []);
  return choices;
};

/** The Font settings: the family picker, bundled fonts first, and the base size. */
export const FontSettings = ({ font, onChange, disabled }: { font: DocumentFont; onChange: (font: DocumentFont) => void; disabled: boolean }) => {
  const choices = useFontChoices();
  const [sizeText, setSizeText] = useState(String(font.size));
  useEffect(() => setSizeText(String(font.size)), [font.size]);
  const bundled = choices.bundled.includes(font.family) || choices.system.includes(font.family) ? choices.bundled : [font.family, ...choices.bundled];
  const setSize = (text: string) => {
    setSizeText(text);
    const size = Number(text);
    if (text.trim() !== "" && Number.isFinite(size) && size >= BASE_SIZES.min && size <= BASE_SIZES.max) onChange({ ...font, size });
  };

  return (
    <fieldset className="workspace-field" disabled={disabled} aria-describedby="formatted-pdf-font-summary">
      <legend className="workspace-panel-label">Font</legend>
      <p id="formatted-pdf-font-summary" className="workspace-note">{fontSummary(font)}</p>
      <div className="formatted-pdf-font-row">
        <div className="workspace-field">
          <Label htmlFor="formatted-pdf-font-family">Family</Label>
          <Select value={font.family} onValueChange={(family) => onChange({ ...font, family })} disabled={disabled}>
            <SelectTrigger id="formatted-pdf-font-family" className="w-full"><SelectValue>{font.family}</SelectValue></SelectTrigger>
            <SelectContent position="popper" aria-label="Font family" className="formatted-pdf-font-list">
              <SelectGroup>
                <SelectLabel>Bundled with Toolbox</SelectLabel>
                {bundled.map((family) => <SelectItem key={family} value={family}>{family}</SelectItem>)}
              </SelectGroup>
              {choices.system.length > 0 && (
                <SelectGroup>
                  <SelectLabel>On this computer</SelectLabel>
                  {choices.system.map((family) => (
                    <SelectItem key={family} value={family} aria-label={`${family}, this computer only`} aria-labelledby={undefined}>
                      {family}<Badge variant="outline" className="ml-auto">This computer only</Badge>
                    </SelectItem>
                  ))}
                </SelectGroup>
              )}
              <p className="workspace-note formatted-pdf-font-hint">
                Bundled fonts look the same on every computer. Fonts from this computer can space text differently on {otherComputer()}.
              </p>
            </SelectContent>
          </Select>
        </div>
        <div className="workspace-field">
          <Label htmlFor="formatted-pdf-font-size">Base size</Label>
          <span className="formatted-pdf-font-size">
            <Input
              id="formatted-pdf-font-size" type="number" inputMode="decimal" min={BASE_SIZES.min} max={BASE_SIZES.max} step={1}
              aria-label="Base size in points" value={sizeText}
              onChange={(event) => setSize(event.target.value)} onBlur={() => setSizeText(String(font.size))}
            />
            <span className="workspace-note" aria-hidden="true">pt</span>
          </span>
        </div>
      </div>
      <p className="workspace-note">Headings scale from the base size. Bold and italic use the same family.</p>
    </fieldset>
  );
};

const styleNames: Record<FontStyle, string> = { regular: "Regular", bold: "Bold", italic: "Italic", boldItalic: "Bold Italic" };
const styleOrder: FontStyle[] = ["regular", "bold", "italic", "boldItalic"];
const sameName = (left: string, right: string) => left.trim().toLowerCase() === right.trim().toLowerCase();

/** One row of the Fonts used table: a requested family, the font that drew it with its styles, and where it came from. */
type FontRow = { requested: string; used: string; styles: FontStyle[]; source: FontUse["source"] };

const fontRows = (fonts: FontUse[]): FontRow[] => {
  const rows: FontRow[] = [];
  for (const font of fonts) {
    const row = rows.find((row) => row.requested === font.requestedFamily && row.used === font.fontUsed && row.source === font.source);
    if (row) { if (!row.styles.includes(font.styleUsed)) row.styles.push(font.styleUsed); }
    else rows.push({ requested: font.requestedFamily, used: font.fontUsed, styles: [font.styleUsed], source: font.source });
  }
  return rows.map((row) => ({ ...row, styles: styleOrder.filter((style) => row.styles.includes(style)) }));
};

const isStandIn = (row: FontRow) => row.source !== "system" && row.source !== "embedded" && !sameName(row.requested, row.used);

const fromLabels = (row: FontRow): { from: string; why: string } => {
  if (row.source === "system") return { from: "This computer", why: "Installed on this computer" };
  if (row.source === "embedded") return { from: "The document", why: "Carried in the document" };
  if (isStandIn(row)) return { from: "Bundled with Toolbox", why: `Stands in for ${row.requested}, which isn't installed here` };
  return { from: "Bundled with Toolbox", why: "The same on every computer" };
};

/** "Header", "Header and page numbers", "Header, footer and page numbers". */
const bandName = (band: { header: string; footer: string; pageNumbers: boolean }) => {
  const parts = [band.header.trim() && "header", band.footer.trim() && "footer", band.pageNumbers && "page numbers"].filter((part): part is string => Boolean(part));
  if (parts.length === 0) return null;
  const list = parts.length === 1 ? parts[0] : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
  return list.replace(/^./, (first) => first.toUpperCase());
};

const Note = ({ title, text }: { title: string; text: string }) => (
  <Card role="note" className="formatted-pdf-font-note py-0">
    <CardContent className="formatted-pdf-font-note-content">
      <Info aria-hidden="true" />
      <span><strong>{title}</strong><span className="workspace-note">{text}</span></span>
    </CardContent>
  </Card>
);

/**
 * After a save, the font that drew each requested family and where it came from, as the board shows. A font from this
 * computer gets the note that the same document can come out differently elsewhere; a stand-in says why it was used.
 */
export const FontsUsed = ({ outcome, band }: { outcome: JobOutcome; band: { header: string; footer: string; pageNumbers: boolean } }) => {
  const rows = fontRows(outcome.fonts ?? []);
  if (outcome.state !== "succeeded" || rows.length === 0) return null;
  const text = rows[0];
  const bands = bandName(band);
  const fromHere = rows.filter((row) => row.source === "system");
  const standIns = rows.filter(isStandIn);
  return (
    <section className="formatted-pdf-fonts-used" aria-labelledby="formatted-pdf-fonts-used-label">
      <h3 id="formatted-pdf-fonts-used-label" className="workspace-panel-label">Fonts used</h3>
      <Card className="py-0">
        <CardContent className="px-0">
          <table aria-labelledby="formatted-pdf-fonts-used-label">
            <thead><tr><th scope="col">You chose</th><th scope="col">Used in the PDF</th><th scope="col">From</th></tr></thead>
            <tbody>
              {rows.map((row) => {
                const { from, why } = fromLabels(row);
                return (
                  <tr key={`${row.requested}\u0000${row.used}\u0000${row.source}`}>
                    <td>{row.requested}</td>
                    <td>{row.used} {row.styles.map((style) => styleNames[style]).join(", ")}</td>
                    <td><span className="formatted-pdf-font-from"><span>{from}</span><span className="workspace-note">{why}</span></span></td>
                  </tr>
                );
              })}
              {bands && <tr><td>{bands}</td><td>{text.used}</td><td>Same as the text</td></tr>}
            </tbody>
          </table>
        </CardContent>
      </Card>
      {fromHere.map((row) => (
        <Note
          key={`here-${row.requested}`}
          title={`${row.requested} came from this computer`}
          text={`Saving the same document on ${otherComputer()} can space or wrap text slightly differently. Pick a bundled font when the PDF must match everywhere.`}
        />
      ))}
      {standIns.map((row) => (
        <Note
          key={`stand-in-${row.requested}`}
          title={`${row.requested} isn't installed, so ${row.used} was used`}
          text={row.source === "bundled fallback"
            ? `${row.used} comes with Toolbox and has the same letter widths, so lines break in the same places. Bundled fonts give the same PDF on every computer.`
            : `${row.used} comes with Toolbox. Lines can break in different places than they would in ${row.requested}. Bundled fonts give the same PDF on every computer.`}
        />
      ))}
    </section>
  );
};
