import { useState, type ComponentType } from 'react';
import { Bold, Check, Info, Italic, TextAlignCenter, TextAlignEnd, TextAlignStart, TriangleAlert } from 'lucide-react';
import { Button } from '../../components/ui/button';
import { Card, CardContent } from '../../components/ui/card';
import { Checkbox } from '../../components/ui/checkbox';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../../components/ui/dialog';
import { Input } from '../../components/ui/input';
import { Label } from '../../components/ui/label';
import { NativeSelect } from '../../components/ui/native-select';
import { RadioGroup, RadioGroupItem } from '../../components/ui/radio-group';
import { ToggleGroup, ToggleGroupItem } from '../../components/ui/toggle-group';
import { useToolAvailability } from '../../ToolAvailabilityContext';
import type { PdfFlattenChoice, PdfFormEntry, PdfFormField, PdfFormInspection } from './contracts';
import { formChoiceText, isFormEntryEmpty } from './form-checks';
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
  signatureFont: string; setSignatureFont: (value: string) => void;
  chooseSignatureImage: () => Promise<void>;
  watermarkText: string; setWatermarkText: (value: string) => void;
  watermarkFontSize: number; setWatermarkFontSize: (value: number) => void;
  watermarkPattern: WatermarkPattern; setWatermarkPattern: (value: WatermarkPattern) => void;
  addFixedWatermark: () => void;
  form: PdfFormInspection | null;
  formValue: (field: PdfFormField) => PdfFormEntry;
  formFlag: (field: PdfFormField) => string | null;
  goToField: (field: PdfFormField) => void;
  goToNextRequired: () => void;
  flatten: PdfFlattenChoice | null; setFlatten: (value: PdfFlattenChoice | null) => void;
  switchToAddText: () => void;
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

const textFonts: TextFontFamily[] = ['Noto Sans', 'Noto Serif', 'Noto Sans Mono'];

const TextFields: Panel = ({ selected, fontSize, setFontSize, textStyle, setTextStyle, updateObject, endGroup, pageNumber }) => {
  const style: TextStyle = selected ? {
    fontFamily: (selected.fontFamily as TextFontFamily | undefined) ?? 'Noto Sans', bold: Boolean(selected.bold), italic: Boolean(selected.italic), align: selected.align ?? 'left',
  } : textStyle;
  const setStyle = (patch: Partial<TextStyle>, field: string) => selected ? updateObject(patch, field) : setTextStyle({ ...textStyle, ...patch });
  const commitRange = (value: string) => { if (selected && value.trim() !== (selected.pageRange ?? '')) updateObject({ pageRange: value.trim() }, 'page-range'); };
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
    {selected?.kind === 'text' && <fieldset className="scene-inspector-field">
      <legend className="mb-1.5 text-sm font-medium">Show this text on</legend>
      <RadioGroup className="gap-1.5" value={selected.pageRange == null ? 'page' : 'pages'}
        onValueChange={(value) => updateObject({ pageRange: value === 'page' ? null : String(pageNumber) }, 'page-range')}>
        <span className="settings-radio"><RadioGroupItem id="scene-text-this-page" value="page" /><Label htmlFor="scene-text-this-page">This page only</Label></span>
        <span className="settings-radio"><RadioGroupItem id="scene-text-pages" value="pages" /><Label htmlFor="scene-text-pages">These pages</Label></span>
      </RadioGroup>
      <Input key={`${selected.id}:${selected.pageRange ?? ''}`} aria-label="Pages for this text box" placeholder="For example 1-3, 7, 10-"
        defaultValue={selected.pageRange ?? ''} disabled={selected.pageRange == null}
        onBlur={(event) => commitRange(event.target.value)} onKeyDown={(event) => { if (event.key === 'Enter') commitRange(event.currentTarget.value); }} />
    </fieldset>}
  </>;
};

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

const TextGuide: Panel = () =>
  <p className="scene-property-note">Click the page to add a text box. Double-click a box or press Enter to edit it.</p>;

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

const pageList = (pages: number[]) => pages.length === 1 ? `page ${pages[0]}`
  : `pages ${pages.slice(0, -1).join(', ')} and ${pages[pages.length - 1]}`;
const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

const flattenChoices: { value: PdfFlattenChoice; label: string }[] = [
  { value: 'both', label: 'Fields and comments' }, { value: 'forms', label: 'Fields only' }, { value: 'annotations', label: 'Comments only' },
];

// Flattening can't be undone in the copy, so turning it on always asks first; turning it off doesn't.
const FlattenSection = ({ form, formValue, flatten, setFlatten }: Pick<SubToolInspectorProps, 'formValue' | 'flatten' | 'setFlatten'> & { form: PdfFormInspection }) => {
  const fields = form.unsupported ? 0 : form.fields.length;
  const comments = form.comments;
  const [choice, setChoice] = useState<PdfFlattenChoice>(fields && comments ? 'both' : fields ? 'forms' : 'annotations');
  const [confirming, setConfirming] = useState(false);
  const formsOnly = !comments || choice === 'forms';
  const things = formsOnly ? 'fields' : choice === 'annotations' ? 'comments' : 'fields and comments';
  const unavailable = !fields && !comments
    ? form.unsupported ? "Toolbox can't read this form's fields, and the PDF has no comments to flatten." : 'This PDF has no form fields or comments to flatten.'
    : null;
  const missing = choice === 'annotations' ? 0 : form.fields.filter((field) => field.required && isFormEntryEmpty(formValue(field))).length;
  const [fieldCount, commentCount] = [choice === 'annotations' ? 0 : fields, choice === 'forms' ? 0 : comments];
  const counted = [fieldCount ? plural(fieldCount, 'field', 'fields') : '', commentCount ? plural(commentCount, 'comment', 'comments') : ''].filter(Boolean).join(' and ');
  const consequences = [
    `${fieldCount + commentCount === 1 ? 'The' : 'All'} ${counted} ${fieldCount + commentCount === 1 ? 'becomes' : 'become'} part of the page in the exported copy. Nobody will be able to change ${fieldCount + commentCount === 1 ? 'it' : 'them'} there.`,
    missing ? `${plural(missing, 'required field is', 'required fields are')} still empty.` : '',
    'You can turn this off before exporting.',
  ].filter(Boolean).join(' ');
  const label = !fields && comments ? 'Flatten comments when exporting' : 'Flatten fields when exporting';
  return <div className="scene-inspector-field border-t pt-3.5">
    <span className="text-sm font-medium" id="scene-flatten">Flatten</span>
    <p className="scene-property-note">{unavailable
      ? <>Flatten is unavailable. {unavailable}</>
      : `Turns the ${fields ? 'fields' : 'comments'} into plain page content in the exported copy, so they can't be changed later.`}</p>
    {fields > 0 && comments > 0 && <RadioGroup className="gap-1.5" aria-label="What to flatten" value={choice} disabled={flatten !== null}
      onValueChange={(value) => setChoice(value as PdfFlattenChoice)}>
      {flattenChoices.map((item) => <span key={item.value} className="settings-radio">
        <RadioGroupItem id={`scene-flatten-${item.value}`} value={item.value} /><Label htmlFor={`scene-flatten-${item.value}`}>{item.label}</Label>
      </span>)}
    </RadioGroup>}
    <span className="settings-radio">
      <Checkbox id="scene-flatten-on" checked={flatten !== null} disabled={Boolean(unavailable)}
        onCheckedChange={(checked) => checked ? setConfirming(true) : setFlatten(null)} />
      <Label htmlFor="scene-flatten-on">{label}</Label>
    </span>
    <Dialog open={confirming} onOpenChange={setConfirming}>
      <DialogContent role="alertdialog" showCloseButton={false} className="sm:max-w-[440px]">
        <DialogHeader>
          <DialogTitle>{`Flatten ${things} in the copy?`}</DialogTitle>
          <DialogDescription>{consequences}</DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button variant="outline" onClick={() => setConfirming(false)}>{`Keep ${choice === 'annotations' || !fields ? 'comments' : 'fields'} editable`}</Button>
          <Button onClick={() => { setFlatten(choice); setConfirming(false); }}>Flatten when exporting</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </div>;
};

const FormGuide: Panel = ({ page, form, formValue, formFlag, goToField, goToNextRequired, flatten, setFlatten, switchToAddText }) => {
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
      <FlattenSection form={form} formValue={formValue} flatten={flatten} setFlatten={setFlatten} />
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
    <FlattenSection form={form} formValue={formValue} flatten={flatten} setFlatten={setFlatten} />
  </>;
};

// Fields edit the selected object or the next new one; Guide shows while the sub-tool is active.
export const subToolInspectors: Partial<Record<SubToolId | SceneTool, { Fields?: Panel; Guide?: Panel }>> = {
  text: { Fields: TextFields, Guide: TextGuide },
  highlight: { Guide: HighlightGuide },
  shape: { Fields: ShapeFields },
  signature: { Fields: SignatureFields, Guide: SignatureGuide },
  watermark: { Fields: WatermarkFields, Guide: WatermarkGuide },
  crop: { Guide: CropGuide },
  form: { Guide: FormGuide },
};
