import { useState } from 'react';
import { ChevronLeftIcon, ChevronRightIcon, Trash2Icon, TriangleAlertIcon } from 'lucide-react';
import { Button } from '../../components/ui/button';
import { Card } from '../../components/ui/card';
import { Checkbox } from '../../components/ui/checkbox';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../../components/ui/dialog';
import { Input } from '../../components/ui/input';
import { Label } from '../../components/ui/label';
import { ToggleGroup, ToggleGroupItem } from '../../components/ui/toggle-group';
import type { PdfScene, SceneRect, SceneRedaction } from './scene';

export const redactionFills = { Black: '#000000', White: '#ffffff' } as const;
export interface ListedMark extends SceneRedaction { pageId: string; page: number; label: string }

export const listMarks = (scene: PdfScene): ListedMark[] => scene.pages.flatMap((page, index) => {
  let area = 0;
  return (page.redactions ?? []).map((mark) => ({ ...mark, pageId: page.id, page: index + 1, label: `page ${index + 1}, ${mark.label ?? `area ${++area}`}` }));
});

/** One search hit on a scene page. A source page shown twice in the scene gives one match per copy. */
export interface TextMatch { pageId: string; page: number; rects: SceneRect[]; before: string; text: string; after: string }
/** `marked` is null while the results are under review, then holds each match's mark ids (empty when left unchecked). */
export interface TextSearch { query: string; matchCase: boolean; matches: TextMatch[]; selected: boolean[]; marked: string[][] | null; lastMarked: number }

const matchCount = (count: number) => `${count} ${count === 1 ? 'match' : 'matches'}`;
export const joinPages = (pages: number[]) => pages.length === 1 ? String(pages[0]) : `${pages.slice(0, -1).join(', ')} and ${pages[pages.length - 1]}`;
const pagesPhrase = (pages: number[]) => `${pages.length === 1 ? 'page' : 'pages'} ${joinPages(pages)}`;
const matchPages = (matches: TextMatch[]) => [...new Set(matches.map((match) => match.page))].sort((a, b) => a - b);
const CONTEXT = 40;

/** The footer line for the current search, following the board's wording. */
export const searchStatus = (search: TextSearch | null) => {
  if (!search) return null;
  const quoted = `“${search.query}”`;
  if (search.marked) return `Marked ${matchCount(search.lastMarked)} for ${quoted}.`;
  if (!search.matches.length) return `No matches for ${quoted}.`;
  return `${matchCount(search.matches.length)} for ${quoted} on ${pagesPhrase(matchPages(search.matches))}.`;
};

function TextSearchPanel({ search, marks, onFind, onSearch, onMark }: {
  search: TextSearch | null; marks: ListedMark[];
  onFind: (query: string, matchCase: boolean) => Promise<void>; onSearch: (search: TextSearch | null) => void;
  onMark: (matches: TextMatch[]) => string[][];
}) {
  const [query, setQuery] = useState(search?.query ?? '');
  const [matchCase, setMatchCase] = useState(search?.matchCase ?? false);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const find = async () => {
    if (!query.trim() || busy) return;
    setBusy(true); setFailure(null);
    try { await onFind(query.trim(), matchCase); }
    catch (reason) { setFailure(String(reason instanceof Error ? reason.message : reason).replace(/^(?:Error:\s*)+/i, '')); }
    finally { setBusy(false); }
  };
  const mark = (chosen: boolean[], previous: string[][] | null) => {
    if (!search) return;
    const ids = onMark(search.matches.filter((_, index) => chosen[index]));
    let next = 0;
    const marked = search.matches.map((_, index) => chosen[index] ? ids[next++] ?? [] : previous?.[index] ?? []);
    onSearch({ ...search, marked, lastMarked: ids.length });
  };
  const live = new Set(marks.map((item) => item.id));
  const unmarked = search?.marked ? search.matches.map((_, index) => !search.marked![index].length || !search.marked![index].every((id) => live.has(id))) : [];
  const selectedCount = search?.selected.filter(Boolean).length ?? 0;
  return <>
    <form className="flex flex-col gap-1.5" onSubmit={(event) => { event.preventDefault(); void find(); }}>
      <Label htmlFor="scene-redaction-search">Find text to redact</Label>
      <div className="flex gap-1.5">
        <Input id="scene-redaction-search" className="h-8 flex-1" value={query} onChange={(event) => setQuery(event.target.value)} />
        <Button type="submit" variant="outline" size="sm" disabled={!query.trim() || busy}>Find</Button>
      </div>
      <div className="flex items-center gap-2">
        <Checkbox id="scene-redaction-match-case" checked={matchCase} onCheckedChange={(checked) => setMatchCase(checked === true)} />
        <Label htmlFor="scene-redaction-match-case" className="font-normal text-muted-foreground">Match case</Label>
      </div>
      {failure && <p className="scene-property-note" role="alert">Search didn't finish. {failure}</p>}
      {search && (search.marked || !search.matches.length) && <div className="flex items-center gap-2">
        <p className="scene-property-note flex-1" role="status">{search.matches.length ? `${matchCount(search.matches.length)}, on ${pagesPhrase(matchPages(search.matches))}` : 'No matches. Text inside pictures isn\'t found.'}</p>
        {search.matches.length > 0 && <Button type="button" variant="outline" size="sm" disabled={!unmarked.some(Boolean)} onClick={() => mark(unmarked, search.marked)}>Mark all matches</Button>}
      </div>}
    </form>
    {search && !search.marked && search.matches.length > 0 && <div className="flex flex-col gap-1.5">
      <p className="text-xs font-semibold" id="scene-redaction-results">{matchCount(search.matches.length)} for “{search.query}”</p>
      <ul className="flex flex-col gap-0.5" aria-labelledby="scene-redaction-results">
        {search.matches.map((match, index) => <li key={index}>
          <Label className="items-start gap-2 rounded-md px-2 py-1.5 font-normal">
            <Checkbox className="mt-0.5" checked={search.selected[index]}
              onCheckedChange={(checked) => onSearch({ ...search, selected: search.selected.map((value, at) => at === index ? checked === true : value) })} />
            <span className="flex min-w-0 flex-col gap-0.5">
              <span className="text-sm">Page {match.page}</span>
              <span className="scene-property-note break-words">{Array.from(match.before).length >= CONTEXT ? '…' : ''}{match.before}<strong>{match.text}</strong>{match.after}{Array.from(match.after).length >= CONTEXT ? '…' : ''}</span>
            </span>
          </Label>
        </li>)}
      </ul>
      <p className="scene-property-note">Uncheck a match to leave it visible. Matches inside pictures aren't found, so check scanned pages by eye.</p>
      <div className="flex flex-wrap gap-1.5">
        <Button size="sm" disabled={!selectedCount} onClick={() => mark(search.selected, null)}>Mark {selectedCount} selected</Button>
        <Button variant="ghost" size="sm" onClick={() => onSearch(null)}>Clear search</Button>
      </div>
    </div>}
  </>;
}

const markedPages = (marks: ListedMark[]) => [...new Set(marks.map((mark) => mark.page))].sort((a, b) => a - b);
const capitalized = (label: string) => label[0].toUpperCase() + label.slice(1);

export function RedactInspector({ marks, selectedId, reviewing, preview, fill, search, onSelect, onRemove, onReview, onPreview, onFill, onFind, onSearch, onMark }: {
  marks: ListedMark[]; selectedId: string | null; reviewing: boolean; preview: boolean; fill: string; search: TextSearch | null;
  onSelect: (mark: ListedMark) => void; onRemove: (ids: string[]) => void; onReview: (reviewing: boolean) => void;
  onPreview: (preview: boolean) => void; onFill: (color: string) => void;
  onFind: (query: string, matchCase: boolean) => Promise<void>; onSearch: (search: TextSearch | null) => void;
  onMark: (matches: TextMatch[]) => string[][];
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
      <TextSearchPanel search={search} marks={marks} onFind={onFind} onSearch={onSearch} onMark={onMark} />
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
