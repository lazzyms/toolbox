import type { ComponentType } from 'react';
import { TypedSignature } from './SceneCanvas';
import { Bold, Italic, TextAlignCenter, TextAlignEnd, TextAlignStart } from 'lucide-react';
import { Button } from '../../components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/card';
import { Input } from '../../components/ui/input';
import { Label } from '../../components/ui/label';
import { NativeSelect } from '../../components/ui/native-select';
import { RadioGroup, RadioGroupItem } from '../../components/ui/radio-group';
import { ToggleGroup, ToggleGroupItem } from '../../components/ui/toggle-group';
import { useToolAvailability } from '../../ToolAvailabilityContext';
import type { PdfEditRun, PdfTextEdit } from './contracts';
import { notEditableReasons, textOverflowWords } from './scene';
import type { SceneObject, ScenePage, SceneShape, SceneTool, SignatureMode, TextAlign, TextFontFamily, TextStyle, WatermarkPattern } from './scene';
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
  watermarkText: string; setWatermarkText: (value: string) => void;
  watermarkFontSize: number; setWatermarkFontSize: (value: number) => void;
  watermarkPattern: WatermarkPattern; setWatermarkPattern: (value: WatermarkPattern) => void;
  addFixedWatermark: () => void;
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
const watermarkFontSizes = [18, 24, 32, 40, 48, 56, 64, 72, 88, 104, 120];

const textFonts: TextFontFamily[] = ['Noto Sans', 'Noto Serif', 'Noto Sans Mono'];

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

const WatermarkFields: Panel = (props) => {
  const { selected, updateObject, endGroup } = props;
  const fontSize = selected?.kind === 'watermark' ? selected.fontSize : props.watermarkFontSize;
  return <>
    <div className="scene-inspector-field"><Label htmlFor="scene-watermark-text">Watermark text</Label>
      <Input id="scene-watermark-text" value={selected?.text ?? props.watermarkText} onChange={(event) => selected ? updateObject({ text: event.target.value }, 'watermark-text') : props.setWatermarkText(event.target.value)} onBlur={endGroup} />
    </div>
    <div className="scene-inspector-field"><Label htmlFor="scene-watermark-size">Watermark font size</Label>
      <NativeSelect id="scene-watermark-size" size="sm" value={String(fontSize)} onChange={(event) => {
        const value = Number(event.target.value); selected ? updateObject({ fontSize: value }, 'watermark-font-size') : props.setWatermarkFontSize(value);
      }}>{watermarkFontSizes.map((value) => <option key={value} value={value}>{value} pt</option>)}</NativeSelect>
    </div>
    <div className="scene-inspector-field"><Label htmlFor="scene-watermark-pattern">Watermark pattern</Label>
      <NativeSelect id="scene-watermark-pattern" size="sm" value={selected?.watermarkPattern ?? props.watermarkPattern}
        onChange={(event) => selected ? updateObject({ watermarkPattern: event.target.value as WatermarkPattern }, 'watermark-pattern') : props.setWatermarkPattern(event.target.value as WatermarkPattern)}>
        <option value="across-page">Across page</option><option value="bottom-right-to-top-left">Bottom-right to top-left</option>
        <option value="top-right-to-bottom-left">Top-right to bottom-left</option><option value="center-horizontal">Centre horizontal</option>
        <option value="center-vertical">Centre vertical</option>
      </NativeSelect>
    </div>
    {selected && <Button variant="outline" size="sm" onClick={props.addFixedWatermark}>Add fixed watermark</Button>}
  </>;
};

const TextGuide: Panel = () =>
  <p className="scene-property-note">Click the page to add a text box. Double-click a box or press Enter to edit it.</p>;

const HighlightGuide: Panel = ({ hasTextRuns }) =>
  <p className="scene-property-note">{hasTextRuns ? 'Drag across selectable PDF text, or draw a highlight area.' : 'Drag across the page to highlight an area.'}</p>;

const SignatureGuide: Panel = ({ selected, signatureMode, signaturePath, signatureText }) => selected ? null
  : <p className="scene-property-note">{signatureMode === 'image' ? signaturePath ? 'Click the page to place the picture, or drag to set its size.' : 'Choose a PNG or JPG before placing it.'
    : signatureText.trim() ? 'Click the page to place the signature, or drag to set its size.' : 'Type a name before placing it.'}</p>;

const WatermarkGuide: Panel = ({ selected, addFixedWatermark }) => {
  const availability = useToolAvailability();
  return availability.allows('pdf-watermark') && !selected ? <>
    <p className="scene-property-note">Fixed placement: choose a pattern and add it to the page.</p>
    <Button size="sm" onClick={addFixedWatermark}>Add fixed watermark</Button>
  </> : null;
};

const CropGuide: Panel = ({ page, commitPage }) => {
  const availability = useToolAvailability();
  return availability.allows('pdf-crop') ? <>
    <p className="scene-property-note">Drag a rectangle on the page to crop.</p>
    <Button variant="outline" size="sm" disabled={!page.crop} onClick={() => { if (availability.allows('pdf-crop')) commitPage({ ...page, crop: null }); }}>Remove crop</Button>
  </> : null;
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
};
