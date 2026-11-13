import { useState, type ComponentType } from 'react';
import { TypedSignature } from './SceneCanvas';
import { Bold, Check, Info, Italic, TextAlignCenter, TextAlignEnd, TextAlignStart, TriangleAlert } from 'lucide-react';
import { Button } from '../../components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/card';
import { Input } from '../../components/ui/input';
import { Label } from '../../components/ui/label';
import { NativeSelect } from '../../components/ui/native-select';
import { RadioGroup, RadioGroupItem } from '../../components/ui/radio-group';
import { Slider } from '../../components/ui/slider';
import { ToggleGroup, ToggleGroupItem } from '../../components/ui/toggle-group';
import { useToolAvailability } from '../../ToolAvailabilityContext';
import type { WatermarkOptions, WatermarkPlacement } from '../../contracts';
import type { PdfEditRun, PdfFormEntry, PdfFormField, PdfFormInspection, PdfTextEdit } from './contracts';
import { formChoiceText, isFormEntryEmpty } from './form-checks';
import { notEditableReasons, pagesInRange, textOverflowWords, WATERMARK_DEFAULT_SIZE } from './scene';
import type { SceneObject, ScenePage, SceneShape, SceneTool, SignatureMode, TextAlign, TextFontFamily, TextStyle } from './scene';
import type { SubToolId } from './subtools';

export interface SubToolInspectorProps {
  page: ScenePage;
  pageNumber: number;
  selected: SceneObject | undefined;
  hasTextRuns: boolean;
  updateObject: (patch: Partial<SceneObject>, field: string) => void;
  commitPage: (page: ScenePage) => void;
  endGroup: () => void;
  fontSize: number; setFontSize: (value: number) => void;
  textStyle: TextStyle; setTextStyle: (value: TextStyle) => void;
  shape: SceneShape; setShape: (value: SceneShape) => void;
  signatureMode: SignatureMode; setSignatureMode: (value: SignatureMode) => void;
  signaturePath: string | null; signaturePreview: string | null;
  signatureText: string; setSignatureText: (value: string) => void;
  signatureTextDraft: string; setSignatureTextDraft: (value: string) => void;
  signatureColor: string; setSignatureColor: (value: string) => void;
  signatureError: string | null;
  chooseSignatureImage: () => Promise<void>;
  usedSignatures: UsedSignature[]; reuseSignature: (value: UsedSignature) => void; clearUsedSignatures: () => void;
  pageCount: number;
  addWatermark: () => void;
  chooseWatermarkPicture: () => Promise<void>;
  watermarkPictureError: string | null;
  form: PdfFormInspection | null;
  formValue: (field: PdfFormField) => PdfFormEntry;
  formFlag: (field: PdfFormField) => string | null;
  goToField: (field: PdfFormField) => void;
  goToNextRequired: () => void;
  switchToAddText: () => void;
  selectedRun: PdfEditRun | undefined;
  textEdits: PdfTextEdit[];
  pageNumberOf: (sourceIndex: number) => number;
  showEdit: (edit: PdfTextEdit) => void;
  revertEdit: (edit: PdfTextEdit) => void;
  chooseTool: (tool: SceneTool) => void;
}
type Panel = ComponentType<SubToolInspectorProps>;

// A signature placed earlier in this app session; see usedSignatures in the workspace.
export interface UsedSignature { mode: SignatureMode; text: string; path: string | null; preview: string | null; color: string }
const inks = [{ value: '#1f1f1f', label: 'Black' }, { value: '#1a3a8f', label: 'Dark blue' }];
const watermarkAngles: Record<string, number> = { rising: -45, horizontal: 0, falling: 45 };
const watermarkPlacements: { value: WatermarkPlacement; label: string }[] = [
  { value: 'center', label: 'Centre' }, { value: 'top-left', label: 'Top left' }, { value: 'top-right', label: 'Top right' },
  { value: 'bottom-left', label: 'Bottom left' }, { value: 'bottom-right', label: 'Bottom right' }, { value: 'tile', label: 'Tile across the page' },
];
const watermarkSizes = [0.05, 0.08, 0.1, 0.12, 0.15, 0.2, 0.25, 0.3];

const textFonts: TextFontFamily[] = ['Noto Sans', 'Noto Serif', 'Noto Sans Mono'];
const watermarkFonts = textFonts.flatMap((family) => [`${family}|regular`, `${family}|bold`]);

const TextFields: Panel = ({ selected, fontSize, setFontSize, textStyle, setTextStyle, updateObject, endGroup, pageNumber }) => {
  const style: TextStyle = selected ? {
    fontFamily: (selected.fontFamily as TextFontFamily | undefined) ?? 'Noto Sans', bold: Boolean(selected.bold), italic: Boolean(selected.italic), align: selected.align ?? 'left',
  } : textStyle;
  const setStyle = (patch: Partial<TextStyle>, field: string) => selected ? updateObject(patch, field) : setTextStyle({ ...textStyle, ...patch });
  return <>
    <div className="scene-inspector-field"><Label htmlFor="scene-text-font">Font</Label>
      <NativeSelect id="scene-text-font" size="sm" value={style.fontFamily} onChange={(event) => setStyle({ fontFamily: event.target.value as TextFontFamily }, 'font-family')}>
        {textFonts.map((family) => <option key={family} value={family}>{family}</option>)}
      </NativeSelect>
    </div>
    <div className="scene-inspector-field"><Label htmlFor="scene-text-size">Size</Label>
      <Input id="scene-text-size" type="number" min="6" max="144" value={selected?.fontSize ?? fontSize}
        onChange={(event) => { const value = Math.max(6, Math.min(144, Number(event.target.value) || 6)); selected ? updateObject({ fontSize: value }, 'font') : setFontSize(value); }} onBlur={endGroup} />
    </div>
    <div className="scene-inspector-row">
      <div className="scene-inspector-field"><span className="text-sm font-medium" id="scene-text-style">Style</span>
        <ToggleGroup type="multiple" variant="outline" size="sm" aria-labelledby="scene-text-style"
          value={[style.bold && 'bold', style.italic && 'italic'].filter((value): value is string => Boolean(value))}
          onValueChange={(value) => setStyle({ bold: value.includes('bold'), italic: value.includes('italic') }, 'font-style')}>
          <ToggleGroupItem value="bold" aria-label="Bold"><Bold /></ToggleGroupItem>
          <ToggleGroupItem value="italic" aria-label="Italic"><Italic /></ToggleGroupItem>
        </ToggleGroup>
      </div>
      <div className="scene-inspector-field"><span className="text-sm font-medium" id="scene-text-align">Align</span>
        <ToggleGroup type="single" variant="outline" size="sm" role="radiogroup" aria-labelledby="scene-text-align" value={style.align}
          onValueChange={(value) => { if (value) setStyle({ align: value as TextAlign }, 'align'); }}>
          <ToggleGroupItem value="left" aria-label="Align left"><TextAlignStart /></ToggleGroupItem>
          <ToggleGroupItem value="center" aria-label="Align centre"><TextAlignCenter /></ToggleGroupItem>
          <ToggleGroupItem value="right" aria-label="Align right"><TextAlignEnd /></ToggleGroupItem>
        </ToggleGroup>
      </div>
    </div>
    {selected?.kind === 'text' && <PageRangeField selected={selected} updateObject={updateObject} pageNumber={pageNumber}
      legend="Show this text on" pagesLabel="These pages" inputLabel="Pages for this text box" placeholder="For example 1-3, 7, 10-" />}
  </>;
};

function PageRangeField({ selected, updateObject, pageNumber, legend, pagesLabel, inputLabel, placeholder }: Pick<SubToolInspectorProps, 'updateObject' | 'pageNumber'>
  & { selected: SceneObject; legend: string; pagesLabel: string; inputLabel: string; placeholder: string }) {
  const id = `scene-${selected.kind}`;
  const commitRange = (value: string) => { if (value.trim() !== (selected.pageRange ?? '')) updateObject({ pageRange: value.trim() }, 'page-range'); };
  return <fieldset className="scene-inspector-field">
    <legend className="mb-1.5 text-sm font-medium">{legend}</legend>
    <RadioGroup className="gap-1.5" value={selected.pageRange == null ? 'page' : 'pages'}
      onValueChange={(value) => updateObject({ pageRange: value === 'page' ? null : String(pageNumber) }, 'page-range')}>
      <span className="settings-radio"><RadioGroupItem id={`${id}-this-page`} value="page" /><Label htmlFor={`${id}-this-page`}>This page only</Label></span>
      <span className="settings-radio"><RadioGroupItem id={`${id}-pages`} value="pages" /><Label htmlFor={`${id}-pages`}>{pagesLabel}</Label></span>
    </RadioGroup>
    <Input key={`${selected.id}:${selected.pageRange ?? ''}`} aria-label={inputLabel} placeholder={placeholder}
      defaultValue={selected.pageRange ?? ''} disabled={selected.pageRange == null}
      onBlur={(event) => commitRange(event.target.value)} onKeyDown={(event) => { if (event.key === 'Enter') commitRange(event.currentTarget.value); }} />
  </fieldset>;
}

const ShapeFields: Panel = ({ selected, shape, setShape, updateObject }) => <div className="scene-inspector-field">
  <Label htmlFor="scene-shape-type">Shape type</Label>
  <NativeSelect id="scene-shape-type" size="sm" value={selected?.shape ?? shape}
    onChange={(event) => selected ? updateObject({ shape: event.target.value as SceneShape }, 'shape') : setShape(event.target.value as SceneShape)}>
    <option value="square">Square</option><option value="round">Round</option><option value="triangle">Triangle</option>
    <option value="line">Line</option><option value="dotted-line">Dotted line</option><option value="arrow-left">Arrow left</option>
    <option value="arrow-right">Arrow right</option><option value="arrow-up">Arrow up</option><option value="arrow-down">Arrow down</option>
  </NativeSelect>
</div>;

const fileName = (path: string) => path.split(/[\\/]/).pop() ?? path;

const SignatureFields: Panel = (props) => {
  const { selected, updateObject, endGroup, signatureTextDraft, pageNumber } = props;
  const selectedSignature = selected?.kind === 'signature' ? selected : null;
  const mode = selectedSignature?.signatureMode ?? props.signatureMode;
  const path = selectedSignature ? selectedSignature.signaturePath : props.signaturePath;
  const preview = selectedSignature ? selectedSignature.signaturePreview : props.signaturePreview;
  const name = selectedSignature ? signatureTextDraft : props.signatureText;
  const ink = selectedSignature?.color ?? props.signatureColor;
  const setInk = (value: string) => selectedSignature ? updateObject({ color: value }, 'color') : props.setSignatureColor(value);
  return <>
    <p className="scene-property-note">Type your name or use a picture of your signature, then click where it goes.</p>
    <p className="scene-property-note" id="scene-signature-note">This adds a picture of a signature. It isn't a certified digital signature.</p>
    <ToggleGroup type="single" variant="outline" size="sm" className="w-full" aria-label="Signature source" value={mode}
      onValueChange={(value) => { if (!value) return; selectedSignature ? updateObject({ signatureMode: value as SignatureMode }, 'signature-mode') : props.setSignatureMode(value as SignatureMode); }}>
      <ToggleGroupItem value="text">Type</ToggleGroupItem>
      <ToggleGroupItem value="image">Picture</ToggleGroupItem>
    </ToggleGroup>
    {mode === 'image' ? <>
      <Button variant="outline" size="sm" onClick={() => void props.chooseSignatureImage()}>Choose a PNG or JPG</Button>
      {props.signatureError && <p role="alert" className="scene-property-note text-destructive">{props.signatureError}</p>}
      {path && <p className="scene-property-note">{fileName(path)}</p>}
      {preview && <img className="scene-signature-mini-preview" alt="Selected signature preview" src={preview} />}
    </> : <>
      <div className="scene-inspector-field"><Label htmlFor="scene-signature-text">Name</Label>
        <Input id="scene-signature-text" value={name} maxLength={200}
          onChange={(event) => selectedSignature ? props.setSignatureTextDraft(event.target.value) : props.setSignatureText(event.target.value)}
          onBlur={() => { if (selectedSignature && signatureTextDraft !== selectedSignature.text) updateObject({ text: signatureTextDraft }, 'signature-text'); endGroup(); }} />
      </div>
      <div role="img" aria-label="Preview" className="scene-signature-preview">
        {name.trim() && <TypedSignature text={name} color={ink} width="100%" height="100%" />}
      </div>
    </>}
    <div className="scene-inspector-field"><span className="text-sm font-medium" id="scene-signature-ink">Ink</span>
      <div className="flex items-center gap-3">
        <RadioGroup className="flex gap-3" aria-labelledby="scene-signature-ink" value={inks.some((item) => item.value === ink.toLowerCase()) ? ink.toLowerCase() : ''} onValueChange={setInk}>
          {inks.map((item) => <RadioGroupItem key={item.value} value={item.value} aria-label={item.label} className="size-5" style={{ backgroundColor: item.value }} />)}
        </RadioGroup>
        <input type="color" aria-label="Other colour" value={ink} onChange={(event) => setInk(event.target.value)} onBlur={endGroup} />
      </div>
    </div>
    <p className="scene-property-note">Corner handles keep proportions.</p>
    {selectedSignature && <PageRangeField selected={selectedSignature} updateObject={updateObject} pageNumber={pageNumber}
      legend="Place on" pagesLabel="These pages, for initials" inputLabel="Pages for this signature" placeholder="For example 1-13" />}
    {props.usedSignatures.length > 0 && <div className="scene-inspector-field">
      <span className="text-sm font-medium" id="scene-signature-reuse">Use again</span>
      <p className="scene-property-note">Signatures you place stay here until you close Toolbox. They're never saved to disk.</p>
      <ul className="flex flex-col gap-1" aria-labelledby="scene-signature-reuse" aria-describedby="scene-signature-note">
        {props.usedSignatures.map((item) => <li key={`${item.mode}:${item.path ?? item.text}:${item.color}`}>
          <Button variant="ghost" size="sm" className="w-full justify-between" onClick={() => props.reuseSignature(item)}>
            <span style={{ color: item.color }}>{item.mode === 'text' ? item.text : fileName(item.path ?? '')}</span>
            <span className="scene-property-note">{item.mode === 'text' ? 'Typed' : fileName(item.path ?? '')}</span>
          </Button>
        </li>)}
      </ul>
      <Button variant="ghost" size="sm" onClick={props.clearUsedSignatures}>Clear list</Button>
    </div>}
  </>;
};

const WatermarkFields: Panel = ({ selected, updateObject, endGroup, pageNumber, pageCount, chooseWatermarkPicture, watermarkPictureError }) => {
  const [rangeError, setRangeError] = useState<string | null>(null);
  const [customAngle, setCustomAngle] = useState(false);
  if (selected?.kind !== 'watermark' || !selected.watermark) return null;
  const options = selected.watermark;
  const picture = options.logoPath !== undefined;
  const update = (patch: Partial<WatermarkOptions>, field: string) => updateObject({ watermark: { ...options, ...patch } }, field);
  const rotation = options.rotation ?? 0;
  const angle = customAngle ? 'custom' : Object.keys(watermarkAngles).find((key) => watermarkAngles[key] === rotation) ?? 'custom';
  const position = typeof options.position === 'object' ? 'moved' : options.position ?? 'center';
  const range = selected.pageRange ?? 'all';
  const rangeChoice = range === 'all' ? 'all' : range === '2-' ? 'skip-first' : 'pages';
  let shownOn: number | null = null;
  try { shownOn = pagesInRange(range, pageCount).length; } catch { /* The range input reports the problem. */ }
  const commitRange = (value: string) => {
    const next = value.trim();
    try { pagesInRange(next, pageCount); } catch (reason) { setRangeError(reason instanceof Error ? reason.message : String(reason)); return; }
    setRangeError(null);
    if (next !== range) updateObject({ pageRange: next }, 'page-range');
  };
  return <>
    <ToggleGroup type="single" variant="outline" size="sm" className="w-full" aria-label="Watermark type" value={picture ? 'picture' : 'text'}
      onValueChange={(value) => {
        if (value === 'text' && picture) updateObject({ watermark: { ...options, logoPath: undefined, size: options.size ?? WATERMARK_DEFAULT_SIZE, text: selected.text || 'DRAFT', color: selected.color }, watermarkPreview: null, watermarkImageSize: null }, 'watermark-type');
        if (value === 'picture') void chooseWatermarkPicture();
      }}>
      <ToggleGroupItem value="text" className="flex-1">Text</ToggleGroupItem>
      <ToggleGroupItem value="picture" className="flex-1">Picture</ToggleGroupItem>
    </ToggleGroup>
    {watermarkPictureError && <p role="alert" className="scene-error">{watermarkPictureError}</p>}
    {picture ? <>
      <Button variant="outline" size="sm" onClick={() => void chooseWatermarkPicture()}>Choose another picture</Button>
      <p className="scene-property-note">{options.logoPath?.split(/[\\/]/).pop()}</p>
    </> : <>
      <div className="scene-inspector-field"><Label htmlFor="scene-watermark-text">Text</Label>
        <Input id="scene-watermark-text" value={options.text ?? ''} onChange={(event) => updateObject({ text: event.target.value, watermark: { ...options, text: event.target.value } }, 'watermark-text')} onBlur={endGroup} />
      </div>
      <div className="scene-inspector-row">
        <div className="scene-inspector-field"><Label htmlFor="scene-watermark-font">Font</Label>
          <NativeSelect id="scene-watermark-font" size="sm" value={`${selected.fontFamily ?? 'Noto Sans'}|${selected.bold ? 'bold' : 'regular'}`}
            onChange={(event) => { const [fontFamily, weight] = event.target.value.split('|'); updateObject({ fontFamily, bold: weight === 'bold' }, 'watermark-font'); }}>
            {watermarkFonts.map((value) => <option key={value} value={value}>{value.replace('|regular', '').replace('|bold', ' Bold')}</option>)}
          </NativeSelect>
        </div>
        <div className="scene-inspector-field"><Label htmlFor="scene-watermark-color">Colour</Label>
          <input id="scene-watermark-color" type="color" value={options.color ?? '#000000'} onChange={(event) => updateObject({ color: event.target.value, watermark: { ...options, color: event.target.value } }, 'watermark-color')} onBlur={endGroup} />
        </div>
      </div>
    </>}
    <div className="scene-inspector-field">
      <div className="scene-inspector-slider-label"><Label id="scene-watermark-opacity-label">Opacity</Label><output>{options.opacity}%</output></div>
      <Slider aria-labelledby="scene-watermark-opacity-label" min={5} max={100} value={[options.opacity]}
        onValueChange={([value]) => { if (value !== undefined) update({ opacity: value }, 'watermark-opacity'); }} onValueCommit={endGroup} />
    </div>
    <div className="scene-inspector-row">
      <div className="scene-inspector-field"><Label htmlFor="scene-watermark-angle">Angle</Label>
        <NativeSelect id="scene-watermark-angle" size="sm" value={angle}
          onChange={(event) => { setCustomAngle(event.target.value === 'custom'); update({ rotation: watermarkAngles[event.target.value] ?? rotation }, 'watermark-angle'); }}>
          <option value="rising">Diagonal, rising</option><option value="horizontal">Horizontal</option>
          <option value="falling">Diagonal, falling</option><option value="custom">Custom</option>
        </NativeSelect>
      </div>
      {angle === 'custom' && <div className="scene-inspector-field"><Label htmlFor="scene-watermark-rotation">Degrees</Label>
        <Input id="scene-watermark-rotation" type="number" min="-180" max="180" value={rotation}
          onChange={(event) => update({ rotation: Math.max(-180, Math.min(180, Number(event.target.value) || 0)) }, 'watermark-angle')} onBlur={endGroup} />
      </div>}
    </div>
    <div className="scene-inspector-row">
      <div className="scene-inspector-field"><Label htmlFor="scene-watermark-position">Position</Label>
        <NativeSelect id="scene-watermark-position" size="sm" value={position}
          onChange={(event) => update({ position: event.target.value as WatermarkPlacement }, 'watermark-position')}>
          {watermarkPlacements.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
          {position === 'moved' && <option value="moved" disabled>Moved by hand</option>}
        </NativeSelect>
      </div>
      <div className="scene-inspector-field"><Label htmlFor="scene-watermark-size">Size</Label>
        <NativeSelect id="scene-watermark-size" size="sm" value={options.size === undefined ? 'original' : String(options.size)}
          onChange={(event) => update({ size: event.target.value === 'original' ? undefined : Number(event.target.value) }, 'watermark-size')}>
          {picture && <option value="original">Original, up to 180 × 100 pt</option>}
          {watermarkSizes.map((value) => <option key={value} value={String(value)}>{Math.round(value * 100)}% of the page</option>)}
        </NativeSelect>
      </div>
    </div>
    <p className="scene-property-note">With the watermark selected, arrow keys move it and Shift+arrow moves it further.</p>
    <div className="scene-inspector-field"><span className="text-sm font-medium" id="scene-watermark-layer">Layer</span>
      <ToggleGroup type="single" variant="outline" size="sm" role="radiogroup" className="w-full" aria-labelledby="scene-watermark-layer" value={selected.layer ?? 'front'}
        onValueChange={(value) => { if (value) updateObject({ layer: value as 'front' | 'behind' }, 'watermark-layer'); }}>
        <ToggleGroupItem value="front" className="flex-1">Above content</ToggleGroupItem>
        <ToggleGroupItem value="behind" className="flex-1">Behind content</ToggleGroupItem>
      </ToggleGroup>
    </div>
    <fieldset className="scene-inspector-field">
      <legend className="mb-1.5 text-sm font-medium">Pages</legend>
      <RadioGroup className="gap-1.5" value={rangeChoice}
        onValueChange={(value) => { setRangeError(null); updateObject({ pageRange: value === 'all' ? 'all' : value === 'skip-first' ? '2-' : String(pageNumber) }, 'page-range'); }}>
        <span className="settings-radio"><RadioGroupItem id="scene-watermark-all" value="all" /><Label htmlFor="scene-watermark-all">All {pageCount} page{pageCount === 1 ? '' : 's'}</Label></span>
        <span className="settings-radio"><RadioGroupItem id="scene-watermark-skip-first" value="skip-first" disabled={pageCount < 2} /><Label htmlFor="scene-watermark-skip-first">Skip the first page</Label></span>
        <span className="settings-radio"><RadioGroupItem id="scene-watermark-pages" value="pages" /><Label htmlFor="scene-watermark-pages">These pages</Label></span>
      </RadioGroup>
      <Input key={`${selected.id}:${range}`} aria-label="Pages to include" placeholder="For example 2-10, 12" aria-invalid={Boolean(rangeError)}
        defaultValue={rangeChoice === 'pages' ? range : ''} disabled={rangeChoice !== 'pages'}
        onBlur={(event) => commitRange(event.target.value)} onKeyDown={(event) => { if (event.key === 'Enter') commitRange(event.currentTarget.value); }} />
      {rangeError && <p role="alert" className="scene-error">{rangeError}</p>}
    </fieldset>
    {shownOn !== null && <p className="scene-property-note">{shownOn === pageCount ? `Watermark added to all ${pageCount} page${pageCount === 1 ? '' : 's'}.` : `Watermark added to ${shownOn} of ${pageCount} pages.`}</p>}
    <p className="scene-property-note">The preview updates as you type. A watermark behind content may be hidden by full-page images.</p>
  </>;
};

const TextGuide: Panel = () =>
  <p className="scene-property-note">Click the page to add a text box. Double-click a box or press Enter to edit it.</p>;

const HighlightGuide: Panel = ({ hasTextRuns }) =>
  <p className="scene-property-note">{hasTextRuns ? 'Drag across selectable PDF text, or draw a highlight area.' : 'Drag across the page to highlight an area.'}</p>;

const SignatureGuide: Panel = ({ selected, signatureMode, signaturePath, signatureText }) => selected ? null
  : <p className="scene-property-note">{signatureMode === 'image' ? signaturePath ? 'Click the page to place the picture, or drag to set its size.' : 'Choose a PNG or JPG before placing it.'
    : signatureText.trim() ? 'Click the page to place the signature, or drag to set its size.' : 'Type a name before placing it.'}</p>;

const WatermarkGuide: Panel = ({ selected, addWatermark }) => {
  const availability = useToolAvailability();
  return availability.allows('pdf-watermark') && !selected ? <>
    <p className="scene-property-note">One watermark, applied to every page in the range. Edit it once, on any page.</p>
    <Button size="sm" onClick={addWatermark}>Add watermark</Button>
  </> : null;
};

const CropGuide: Panel = ({ page, commitPage }) => {
  const availability = useToolAvailability();
  return availability.allows('pdf-crop') ? <>
    <p className="scene-property-note">Drag a rectangle on the page to crop.</p>
    <Button variant="outline" size="sm" disabled={!page.crop} onClick={() => { if (availability.allows('pdf-crop')) commitPage({ ...page, crop: null }); }}>Remove crop</Button>
  </> : null;
};

const pageList = (pages: number[]) => pages.length === 1 ? `page ${pages[0]}`
  : `pages ${pages.slice(0, -1).join(', ')} and ${pages[pages.length - 1]}`;
const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

const FormGuide: Panel = ({ page, form, formValue, formFlag, goToField, goToNextRequired, switchToAddText }) => {
  if (!form) return <p className="scene-property-note" role="status">Reading the form fields…</p>;
  if (form.unsupported || !form.fields.length) {
    const [headline, ...rest] = (form.unsupported ?? 'This PDF has no form fields to fill.').split(/(?<=\.) /);
    return <>
      <p className="scene-property-note">{form.unsupported ? 'This PDF has a form, but not one Toolbox can fill.' : 'Fill form is unavailable for this PDF.'}</p>
      <Card className="scene-form-note py-0" role="note"><CardContent className="scene-form-note-content">
        <TriangleAlert aria-hidden="true" /><p><strong>{headline}</strong> {rest.join(' ')}</p>
      </CardContent></Card>
      <p className="scene-property-note">{form.unsupported ? 'To fill it anyway, open it in Adobe Acrobat or Reader. Or type your answers onto the page with Add text.' : 'Type your answers onto the page with Add text instead.'}</p>
      <Button variant="outline" size="sm" className="self-start" onClick={switchToAddText}>Use Add text instead</Button>
    </>;
  }
  const pages = [...new Set(form.fields.map((field) => field.widgets[0].page + 1))].sort((a, b) => a - b);
  const missing = form.fields.filter((field) => field.required && isFormEntryEmpty(formValue(field))).length;
  const invalid = form.fields.filter((field) => (formFlag(field) ?? 'Required') !== 'Required').length;
  const status = (field: PdfFormField) => {
    const value = formValue(field);
    const flag = formFlag(field);
    const flagged = (text: string) => flag ? <span className="text-destructive">{text}</span> : text;
    if (flag && flag !== 'Required') return flagged(flag);
    if (field.type === 'checkbox') return value ? 'Checked' : field.required ? flagged('Required, not checked') : null;
    if (!isFormEntryEmpty(value)) return field.type === 'text' ? <Check aria-label="Filled" /> : formChoiceText(field, value);
    if (field.widgets[0].page !== page.sourceIndex) return `Page ${field.widgets[0].page + 1}${field.required ? ', required' : ''}`;
    return field.required ? flagged('Required, empty') : field.readOnly ? 'Read-only' : null;
  };
  const problems = [
    invalid ? `${plural(invalid, "field has an entry that isn't valid", "fields have entries that aren't valid")}` : '',
    missing ? `${plural(missing, 'required field is', 'required fields are')} still empty` : '',
  ].filter(Boolean).join(', and ');
  return <>
    <p className="scene-property-note">Type into the fields on the page. Tab moves to the next field in the order below.</p>
    <Card className="scene-form-note py-0" role="note"><CardContent className="scene-form-note-content">
      <Info aria-hidden="true" /><p>{`${plural(form.fields.length, 'field', 'fields')} on ${pageList(pages)}.`}{problems ? ` ${problems}.` : ''}</p>
    </CardContent></Card>
    <Button variant="outline" size="sm" className="self-start" disabled={!missing} onClick={goToNextRequired}>Go to next empty required field</Button>
    <div className="scene-inspector-field"><span className="text-sm font-medium" id="scene-form-fields">Fields</span>
      <ol className="scene-form-fields" aria-labelledby="scene-form-fields">
        {form.fields.map((field) => <li key={field.name}>
          <Button variant="ghost" size="sm" className="w-full justify-between" onClick={() => goToField(field)}>
            <span className="truncate">{field.label || field.name}</span><span className="scene-property-note">{status(field)}</span>
          </Button>
        </li>)}
      </ol>
    </div>
  </>;
};

const EditTextGuide: Panel = ({ page, selectedRun, textEdits, pageNumberOf, showEdit, revertEdit, chooseTool }) => {
  const availability = useToolAvailability();
  const selectedEdit = selectedRun && textEdits.find((edit) => edit.pageIndex === page.sourceIndex && edit.run === selectedRun.index);
  const overflow = selectedEdit ? textOverflowWords(selectedEdit.original, selectedEdit.replacement) : 0;
  return <>
    {selectedRun?.reason && <Card role="alert" className="gap-2 py-3">
      <CardHeader className="gap-1 px-3"><CardTitle className="text-sm">This text can't be edited.</CardTitle>
        <CardDescription>{notEditableReasons[selectedRun.reason].detail}</CardDescription></CardHeader>
      {availability.allows('pdf-edit') && <CardContent className="px-3">
        <Button variant="ghost" size="sm" onClick={() => chooseTool('text')}>Cover it with Add text instead</Button>
      </CardContent>}
    </Card>}
    {selectedRun && !selectedRun.reason && <div className="scene-inspector-field" role="group" aria-label="Selected text">
      <Label>Selected text</Label>
      <p className="scene-property-note">{selectedRun.fontName} · {Math.round(selectedRun.fontSize * 10) / 10} pt</p>
      <p className="scene-property-note">Edited text keeps this font, size and colour.</p>
    </div>}
    {overflow > 0 && <Card role="alert" className="gap-2 py-3">
      <CardHeader className="gap-1 px-3"><CardTitle className="text-sm">This text is too long for its space.</CardTitle>
        <CardDescription>Shorten it so it fits the original line.</CardDescription></CardHeader>
    </Card>}
    {textEdits.length > 0 && <div className="scene-inspector-field" role="group" aria-label="Edits on this document">
      <Label>Edits on this document</Label>
      {textEdits.map((edit) => <Button key={`${edit.pageIndex}-${edit.run}`} variant="ghost" size="sm" className="h-auto justify-start whitespace-normal text-left"
        onClick={() => showEdit(edit)}>Page {pageNumberOf(edit.pageIndex)}: “{edit.original}” to “{edit.replacement}”</Button>)}
    </div>}
    {selectedEdit && <Button variant="ghost" size="sm" onClick={() => revertEdit(selectedEdit)}>Revert this text</Button>}
  </>;
};

// Fields edit the selected object or the next new one; Guide shows while the sub-tool is active.
export const subToolInspectors: Partial<Record<SubToolId | SceneTool, { Fields?: Panel; Guide?: Panel }>> = {
  'edit-text': { Guide: EditTextGuide },
  text: { Fields: TextFields, Guide: TextGuide },
  highlight: { Guide: HighlightGuide },
  shape: { Fields: ShapeFields },
  signature: { Fields: SignatureFields, Guide: SignatureGuide },
  watermark: { Fields: WatermarkFields, Guide: WatermarkGuide },
  crop: { Guide: CropGuide },
  form: { Guide: FormGuide },
};
