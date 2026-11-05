import type { ComponentType } from 'react';
import { Button } from '../../components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/card';
import { Input } from '../../components/ui/input';
import { Label } from '../../components/ui/label';
import { NativeSelect } from '../../components/ui/native-select';
import { useToolAvailability } from '../../ToolAvailabilityContext';
import type { PdfEditRun, PdfTextEdit } from './contracts';
import { notEditableReasons, textOverflowWords } from './scene';
import type { SceneObject, ScenePage, SceneShape, SceneTool, SignatureMode, WatermarkPattern } from './scene';
import type { SubToolId } from './subtools';

export interface SubToolInspectorProps {
  page: ScenePage;
  selected: SceneObject | undefined;
  hasTextRuns: boolean;
  updateObject: (patch: Partial<SceneObject>, field: string) => void;
  commitPage: (page: ScenePage) => void;
  endGroup: () => void;
  fontSize: number; setFontSize: (value: number) => void;
  shape: SceneShape; setShape: (value: SceneShape) => void;
  signatureMode: SignatureMode; setSignatureMode: (value: SignatureMode) => void;
  signaturePath: string | null; signaturePreview: string | null;
  signatureText: string; setSignatureText: (value: string) => void;
  signatureTextDraft: string; setSignatureTextDraft: (value: string) => void;
  signatureFont: string; setSignatureFont: (value: string) => void;
  chooseSignatureImage: () => Promise<void>;
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

const signatureFontOptions = [
  { value: 'Satisfy', label: 'Satisfy — cursive' },
  { value: 'Pacifico', label: 'Pacifico — cursive' },
  { value: 'Helvetica-Oblique', label: 'Helvetica italic' },
  { value: 'Times-Italic', label: 'Times italic' },
  { value: 'Courier-Oblique', label: 'Courier italic' },
  { value: 'Helvetica', label: 'Helvetica' },
  { value: 'Times-Roman', label: 'Times' },
  { value: 'Courier', label: 'Courier' },
];
const watermarkFontSizes = [18, 24, 32, 40, 48, 56, 64, 72, 88, 104, 120];

const TextFields: Panel = ({ selected, fontSize, setFontSize, updateObject, endGroup }) => <div className="scene-inspector-field">
  <Label htmlFor="scene-text-size">Font size</Label>
  <Input id="scene-text-size" type="number" min="6" max="144" value={selected?.fontSize ?? fontSize}
    onChange={(event) => { const value = Math.max(6, Math.min(144, Number(event.target.value) || 6)); selected ? updateObject({ fontSize: value }, 'font') : setFontSize(value); }} onBlur={endGroup} />
</div>;

const ShapeFields: Panel = ({ selected, shape, setShape, updateObject }) => <div className="scene-inspector-field">
  <Label htmlFor="scene-shape-type">Shape type</Label>
  <NativeSelect id="scene-shape-type" size="sm" value={selected?.shape ?? shape}
    onChange={(event) => selected ? updateObject({ shape: event.target.value as SceneShape }, 'shape') : setShape(event.target.value as SceneShape)}>
    <option value="square">Square</option><option value="round">Round</option><option value="triangle">Triangle</option>
    <option value="line">Line</option><option value="dotted-line">Dotted line</option><option value="arrow-left">Arrow left</option>
    <option value="arrow-right">Arrow right</option><option value="arrow-up">Arrow up</option><option value="arrow-down">Arrow down</option>
  </NativeSelect>
</div>;

const SignatureFields: Panel = (props) => {
  const { selected, updateObject, endGroup, signatureTextDraft } = props;
  const selectedSignature = selected?.kind === 'signature' ? selected : null;
  const mode = selected?.signatureMode ?? props.signatureMode;
  const path = selected?.signaturePath ?? props.signaturePath;
  const preview = selected?.signaturePreview ?? props.signaturePreview;
  return <>
    <div className="scene-inspector-field"><Label htmlFor="scene-signature-mode">Signature mode</Label>
      <NativeSelect id="scene-signature-mode" size="sm" value={mode}
        onChange={(event) => { const value = event.target.value as SignatureMode; selected ? updateObject({ signatureMode: value, strokes: [] }, 'signature-mode') : props.setSignatureMode(value); }}>
        <option value="image">Choose image</option><option value="text">Type signature</option>
      </NativeSelect>
    </div>
    {mode === 'image' ? <>
      <Button variant="outline" size="sm" onClick={() => void props.chooseSignatureImage()}>Choose signature image</Button>
      <p className="scene-property-note">{path ? path.split(/[\\/]/).pop() : 'No image chosen'}</p>
    </> : <>
      <div className="scene-inspector-field"><Label htmlFor="scene-signature-text">Signature text</Label>
        <Input id="scene-signature-text" value={selectedSignature ? signatureTextDraft : props.signatureText}
          onChange={(event) => selectedSignature ? props.setSignatureTextDraft(event.target.value) : props.setSignatureText(event.target.value)}
          onBlur={() => { if (selectedSignature && signatureTextDraft !== selectedSignature.text) updateObject({ text: signatureTextDraft }, 'signature-text'); endGroup(); }} />
      </div>
      <div className="scene-inspector-field"><Label htmlFor="scene-signature-font">Signature font</Label>
        <NativeSelect id="scene-signature-font" size="sm" value={selected?.fontFamily ?? props.signatureFont}
          onChange={(event) => selected ? updateObject({ fontFamily: event.target.value }, 'signature-font') : props.setSignatureFont(event.target.value)}>
          {signatureFontOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
        </NativeSelect>
      </div>
    </>}
    <div className="scene-inspector-field"><Label htmlFor="scene-signature-size">Font size</Label>
      <Input id="scene-signature-size" type="number" min="8" max="144" value={selected?.fontSize ?? Math.max(18, props.fontSize)}
        onChange={(event) => { const value = Math.max(8, Math.min(144, Number(event.target.value) || 8)); selected ? updateObject({ fontSize: value }, 'signature-font-size') : props.setFontSize(value); }} onBlur={endGroup} />
    </div>
    {mode === 'image' && preview && <img className="scene-signature-mini-preview" alt="Selected signature preview" src={preview} />}
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

const HighlightGuide: Panel = ({ hasTextRuns }) =>
  <p className="scene-property-note">{hasTextRuns ? 'Drag across selectable PDF text, or draw a highlight area.' : 'Drag across the page to highlight an area.'}</p>;

const SignatureGuide: Panel = ({ selected, signatureMode }) => selected ? null
  : <p className="scene-property-note">{signatureMode === 'image' ? 'Choose an image, then drag its box onto the page.' : 'Type a signature, choose its font, then drag a box onto the page.'}</p>;

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
  text: { Fields: TextFields },
  highlight: { Guide: HighlightGuide },
  shape: { Fields: ShapeFields },
  signature: { Fields: SignatureFields, Guide: SignatureGuide },
  watermark: { Fields: WatermarkFields, Guide: WatermarkGuide },
  crop: { Guide: CropGuide },
};
