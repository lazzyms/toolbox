import { useState } from 'react';
import { ChevronLeftIcon, ChevronRightIcon, Trash2Icon, TriangleAlertIcon } from 'lucide-react';
import { Button } from '../../components/ui/button';
import { Card } from '../../components/ui/card';
import { Checkbox } from '../../components/ui/checkbox';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../../components/ui/dialog';
import { Label } from '../../components/ui/label';
import { ToggleGroup, ToggleGroupItem } from '../../components/ui/toggle-group';
import type { PdfScene, SceneRedaction } from './scene';

export const redactionFills = { Black: '#000000', White: '#ffffff' } as const;
export interface ListedMark extends SceneRedaction { pageId: string; page: number; label: string }

export const listMarks = (scene: PdfScene): ListedMark[] => scene.pages.flatMap((page, index) =>
  (page.redactions ?? []).map((mark, order) => ({ ...mark, pageId: page.id, page: index + 1, label: `page ${index + 1}, area ${order + 1}` })));

const joinPages = (pages: number[]) => pages.length === 1 ? String(pages[0]) : `${pages.slice(0, -1).join(', ')} and ${pages[pages.length - 1]}`;
const markedPages = (marks: ListedMark[]) => [...new Set(marks.map((mark) => mark.page))].sort((a, b) => a - b);
const capitalized = (label: string) => label[0].toUpperCase() + label.slice(1);

export function RedactInspector({ marks, selectedId, reviewing, preview, fill, onSelect, onRemove, onReview, onPreview, onFill }: {
  marks: ListedMark[]; selectedId: string | null; reviewing: boolean; preview: boolean; fill: string;
  onSelect: (mark: ListedMark) => void; onRemove: (ids: string[]) => void; onReview: (reviewing: boolean) => void;
  onPreview: (preview: boolean) => void; onFill: (color: string) => void;
}) {
  const current = marks.findIndex((mark) => mark.id === selectedId);
  const step = (delta: number) => { if (marks.length) onSelect(marks[(Math.max(current, 0) + delta + marks.length) % marks.length]); };
  const list = <ul className="flex flex-col gap-0.5" aria-label="Marks">
    {marks.map((mark) => <li key={mark.id}>
      <Button variant={mark.id === selectedId ? 'secondary' : 'ghost'} size="sm" className="w-full justify-start" aria-current={mark.id === selectedId}
        onClick={() => onSelect(mark)}>{capitalized(mark.label)}</Button>
    </li>)}
  </ul>;
  return <>
    {reviewing && marks.length ? <>
      <p className="text-xs font-semibold" role="status">Mark {Math.max(current, 0) + 1} of {marks.length}: {marks[Math.max(current, 0)].label}</p>
      <div className="flex flex-wrap gap-1.5">
        <Button variant="outline" size="sm" onClick={() => step(-1)}><ChevronLeftIcon />Previous</Button>
        <Button variant="outline" size="sm" onClick={() => step(1)}>Next<ChevronRightIcon /></Button>
        <Button variant="outline" size="sm" disabled={current < 0} onClick={() => onRemove([marks[current].id])}><Trash2Icon />Remove</Button>
      </div>
      <div className="flex items-center gap-2">
        <Checkbox id="scene-redaction-preview" checked={preview} onCheckedChange={(checked) => onPreview(checked === true)} />
        <Label htmlFor="scene-redaction-preview">Preview as exported</Label>
      </div>
      <p className="scene-property-note">Marks show as they will in the copy: solid boxes with nothing underneath. Tab and Shift+Tab also step through marks, and the page follows.</p>
      {list}
      <Button variant="secondary" size="sm" className="self-start" onClick={() => onReview(false)}>Done reviewing</Button>
    </> : <>
      <p className="text-xs font-semibold">Marks</p>
      {marks.length ? list : <p className="scene-property-note">No marks yet.</p>}
      <div className="flex flex-wrap gap-1.5">
        <Button variant="outline" size="sm" disabled={!marks.length} onClick={() => onReview(true)}>Review marks</Button>
        <Button variant="outline" size="sm" disabled={current < 0} onClick={() => onRemove([marks[current].id])}><Trash2Icon />Remove mark</Button>
      </div>
      <Button variant="ghost" size="sm" className="self-start" disabled={!marks.length} onClick={() => onRemove(marks.map((mark) => mark.id))}>Remove all</Button>
    </>}
    <div className="scene-inspector-field">
      <Label id="scene-redaction-fill">Fill</Label>
      <ToggleGroup type="single" variant="outline" size="sm" aria-labelledby="scene-redaction-fill" value={fill}
        onValueChange={(value) => { if (value) onFill(value); }}>
        {Object.entries(redactionFills).map(([label, color]) => <ToggleGroupItem key={color} value={color}>{label}</ToggleGroupItem>)}
      </ToggleGroup>
    </div>
    <Card role="note" className="flex-row gap-2.5 px-3 py-2.5 text-xs">
      <TriangleAlertIcon className="size-4 shrink-0" aria-hidden="true" />
      <p><strong>Redaction is permanent in the copy.</strong> Pages with marks become images, and hidden data is removed. Details appear before you export.</p>
    </Card>
  </>;
}

export function RedactDisclosure({ marks, open, onOpenChange, onReview, onContinue }: {
  marks: ListedMark[]; open: boolean; onOpenChange: (open: boolean) => void; onReview: () => void; onContinue: () => void;
}) {
  const [checked, setChecked] = useState(false);
  const pages = markedPages(marks);
  const one = pages.length === 1;
  return <Dialog open={open} onOpenChange={(next) => { if (!next) setChecked(false); onOpenChange(next); }}>
    {open && <DialogContent showCloseButton={false} className="sm:max-w-[540px]">
      <DialogHeader>
        <DialogTitle>Before you export with redactions</DialogTitle>
        <DialogDescription>
          {marks.length} {marks.length === 1 ? 'mark' : 'marks'} on {one ? 'page' : 'pages'} {joinPages(pages)} will be removed for good in the copy. To make sure nothing can be recovered:
        </DialogDescription>
      </DialogHeader>
      <ul className="list-disc space-y-1 pl-5 text-sm text-muted-foreground">
        <li>{one ? `Page ${pages[0]} becomes a 300 dpi image. Its text can't be selected or searched.` : `Pages ${joinPages(pages)} become 300 dpi images. Their text can't be selected or searched.`}</li>
        <li>Comments, form fields and hidden layers on {one ? 'that page are' : 'those pages are'} removed.</li>
        <li>Document metadata, XMP data and attachments are removed from the whole copy.</li>
      </ul>
      <div className="flex items-center gap-2">
        <Checkbox id="scene-redaction-checked" checked={checked} onCheckedChange={(value) => setChecked(value === true)} />
        <Label htmlFor="scene-redaction-checked">I've checked the marks on every page</Label>
      </div>
      <DialogFooter>
        <Button variant="outline" onClick={() => { setChecked(false); onReview(); }}>Review marks</Button>
        <Button disabled={!checked} onClick={() => { setChecked(false); onContinue(); }}>Continue to export</Button>
      </DialogFooter>
    </DialogContent>}
  </Dialog>;
}
