import { forwardRef, useEffect, useImperativeHandle, useRef, type KeyboardEvent } from 'react';
import { cn } from 'cn';
import { badgeVariants } from '../../components/ui/badge';

export type TokenName = 'page' | 'pages' | 'date' | 'file';
export const tokenLabels: Record<TokenName, string> = { page: 'Page number', pages: 'Page count', date: 'Date', file: 'File name' };
const tokenPattern = /\{(page|pages|date|file)\}/g;

export interface TokenFieldHandle { insert: (token: TokenName) => void }

interface TokenFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  onFocus: () => void;
  onBlur: () => void;
}

function serialize(node: Node): string {
  let text = '';
  node.childNodes.forEach((child) => {
    if (child.nodeType === Node.TEXT_NODE) text += (child.textContent ?? '').replace(/[\n\u00a0]/g, ' ');
    else if (child instanceof HTMLElement && child.dataset.token) text += `{${child.dataset.token}}`;
    else if (child instanceof HTMLElement && child.tagName !== 'BR') text += serialize(child);
  });
  return text;
}

function tag(token: TokenName) {
  const element = document.createElement('span');
  element.dataset.token = token;
  element.contentEditable = 'false';
  element.className = cn(badgeVariants({ variant: 'secondary' }), 'mx-0.5 align-baseline');
  element.textContent = tokenLabels[token];
  return element;
}

function render(root: HTMLElement, value: string) {
  root.replaceChildren();
  let last = 0;
  for (const match of value.matchAll(tokenPattern)) {
    if (match.index > last) root.append(value.slice(last, match.index));
    root.append(tag(match[1] as TokenName));
    last = match.index + match[0].length;
  }
  if (last < value.length) root.append(value.slice(last));
}

function tokenBeforeCaret(root: HTMLElement): HTMLElement | null {
  const selection = window.getSelection();
  if (!selection || selection.rangeCount === 0 || !selection.isCollapsed) return null;
  const { startContainer, startOffset } = selection.getRangeAt(0);
  if (!root.contains(startContainer)) return null;
  const before = startContainer === root ? root.childNodes[startOffset - 1]
    : startContainer.nodeType === Node.TEXT_NODE && startOffset === 0 ? startContainer.previousSibling : null;
  return before instanceof HTMLElement && before.dataset.token ? before : null;
}

export const TokenField = forwardRef<TokenFieldHandle, TokenFieldProps>(({ label, value, onChange, onFocus, onBlur }, ref) => {
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => { if (root.current && serialize(root.current) !== value) render(root.current, value); }, [value]);
  const emit = () => {
    if (!root.current) return;
    const next = serialize(root.current);
    if (next === '') root.current.replaceChildren();
    onChange(next);
  };
  useImperativeHandle(ref, () => ({
    insert(token) {
      const field = root.current;
      if (!field) return;
      const element = tag(token);
      const selection = window.getSelection();
      const range = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
      if (range && field.contains(range.startContainer)) {
        range.deleteContents();
        range.insertNode(element);
      } else field.append(element);
      const caret = document.createRange();
      caret.setStartAfter(element);
      caret.collapse(true);
      selection?.removeAllRanges();
      selection?.addRange(caret);
      emit();
    },
  }));
  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Enter') { event.preventDefault(); return; }
    if (event.key !== 'Backspace' || !root.current) return;
    const before = tokenBeforeCaret(root.current);
    if (!before) return;
    event.preventDefault();
    before.remove();
    emit();
  };
  return <div ref={root} role="textbox" aria-label={label} aria-multiline="false" contentEditable suppressContentEditableWarning tabIndex={0}
    data-placeholder="Empty"
    className={cn('min-h-9 w-full min-w-0 overflow-hidden rounded-md border border-input bg-transparent px-3 py-1.5 text-sm leading-6 whitespace-nowrap shadow-xs outline-none dark:bg-input/30',
      'focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50',
      'empty:before:text-muted-foreground empty:before:content-[attr(data-placeholder)]')}
    onInput={emit} onKeyDown={keyDown} onFocus={onFocus} onBlur={onBlur}
    onPaste={(event) => { event.preventDefault(); document.execCommand('insertText', false, event.clipboardData.getData('text/plain').replace(/\s+/g, ' ')); }} />;
});
TokenField.displayName = 'TokenField';
