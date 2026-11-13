import { useContext, useEffect, useRef, useState, type ClipboardEvent, type KeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowDown, ArrowUp, Trash2 } from "lucide-react";
import type { JobOutcome, ToolDefinition } from "../contracts";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { Textarea } from "@/components/ui/textarea";
import { ResultList } from "../components/ResultList";
import { RunStateContext } from "../components/ToolScaffold";
import { ShortcutHint } from "../components/ShortcutHint";
import { chooseCreatedDocumentSavePath } from "../components/saveStep";
import { failedOutcome, parseToolResult } from "../jobOutcome";
import { shortcutTarget } from "../shortcuts";

export type BlockKind = "heading1" | "heading2" | "heading3" | "paragraph";

type Block = { id: number; kind: BlockKind; text: string };

const kindLabels: Record<BlockKind, string> = { heading1: "Heading 1", heading2: "Heading 2", heading3: "Heading 3", paragraph: "Paragraph" };
const gutterLabels: Record<BlockKind, string> = { heading1: "H1", heading2: "H2", heading3: "H3", paragraph: "Text" };

/** The defaults this first version always uses; page setup and the font picker come later. */
const PAGE = { pageSize: "a4", margins: "normal" } as const;
const FONT = { family: "Liberation Serif", size: 11 } as const;

let lastBlockId = 0;
const newBlock = (kind: BlockKind = "paragraph", text = ""): Block => ({ id: ++lastBlockId, kind, text });

const isEmpty = (blocks: readonly Block[]) => blocks.every((block) => block.text.trim() === "");

/** Pasted text with blank lines between paragraphs becomes one paragraph block for each. */
const pastedParagraphs = (text: string) => text.replace(/\r\n?/g, "\n").split(/\n[ \t]*\n+/).map((part) => part.trim()).filter(Boolean);

export const FormattedPdfView = ({ utility }: { utility: ToolDefinition }) => {
  const [blocks, setBlocks] = useState<Block[]>(() => [newBlock()]);
  const [selected, setSelected] = useState(0);
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<JobOutcome[] | null>(null);
  const list = useRef<HTMLOListElement>(null);
  const focusNext = useRef<{ id: number; caret: number } | null>(null);
  const reportRunning = useContext(RunStateContext);

  useEffect(() => {
    const next = focusNext.current;
    if (!next) return;
    focusNext.current = null;
    const field = list.current?.querySelector<HTMLTextAreaElement>(`textarea[data-block-id="${next.id}"]`);
    field?.focus();
    field?.setSelectionRange(next.caret, next.caret);
  }, [blocks]);

  const current = blocks[Math.min(selected, blocks.length - 1)];
  const empty = isEmpty(blocks);

  const update = (index: number, change: Partial<Omit<Block, "id">>) =>
    setBlocks((all) => all.map((block, at) => at === index ? { ...block, ...change } : block));

  const move = (index: number, by: -1 | 1) => {
    const target = index + by;
    if (target < 0 || target >= blocks.length) return;
    setBlocks((all) => {
      const next = [...all];
      [next[index], next[target]] = [next[target], next[index]];
      return next;
    });
    setSelected(target);
  };

  const remove = (index: number) => {
    const next = blocks.length === 1 ? [newBlock()] : blocks.filter((_, at) => at !== index);
    const focus = next[Math.min(index, next.length - 1)];
    focusNext.current = { id: focus.id, caret: focus.text.length };
    setBlocks(next);
    setSelected(Math.min(index, next.length - 1));
  };

  /** Enter ends the block at the caret and starts a paragraph; Shift+Enter keeps a line break inside the block. */
  const splitOnEnter = (index: number, event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key !== "Enter" || event.shiftKey || event.metaKey || event.ctrlKey || event.altKey || event.nativeEvent.isComposing) return;
    event.preventDefault();
    const field = event.currentTarget;
    const block = blocks[index];
    const before = block.text.slice(0, field.selectionStart);
    const after = block.text.slice(field.selectionEnd);
    const added = newBlock("paragraph", after);
    focusNext.current = { id: added.id, caret: 0 };
    setBlocks((all) => [...all.slice(0, index), { ...block, text: before }, added, ...all.slice(index + 1)]);
    setSelected(index + 1);
  };

  const splitOnPaste = (index: number, event: ClipboardEvent<HTMLTextAreaElement>) => {
    const parts = pastedParagraphs(event.clipboardData.getData("text/plain"));
    if (parts.length < 2) return;
    event.preventDefault();
    const field = event.currentTarget;
    const block = blocks[index];
    const before = block.text.slice(0, field.selectionStart);
    const after = block.text.slice(field.selectionEnd);
    const added = parts.slice(1).map((text) => newBlock("paragraph", text));
    const last = added[added.length - 1];
    focusNext.current = { id: last.id, caret: last.text.length };
    added[added.length - 1] = { ...last, text: `${last.text}${after}` };
    setBlocks((all) => [...all.slice(0, index), { ...block, text: `${before}${parts[0]}` }, ...added, ...all.slice(index + 1)]);
    setSelected(index + added.length);
  };

  const save = async () => {
    if (empty || saving) return;
    const name = `${title.trim() || "Untitled"}.pdf`;
    const outputPath = await chooseCreatedDocumentSavePath({ title: "Save the PDF", extension: "pdf" }, name, "A4, Liberation Serif 11 pt");
    if (!outputPath) return;
    setSaving(true);
    reportRunning?.(true);
    try {
      const request = {
        document: {
          title: title.trim(),
          author: author.trim(),
          blocks: blocks.filter((block) => block.text.trim() !== "").map(({ kind, text }) => ({ kind, text })),
          page: PAGE,
          font: FONT,
        },
        outputPath,
      };
      setSaved(parseToolResult(await invoke("create_formatted_pdf", { request })));
    } catch (error) {
      setSaved([failedOutcome(outputPath, "processing", error instanceof Error ? error.message : String(error))]);
    } finally {
      setSaving(false);
      reportRunning?.(false);
    }
  };

  const startOver = () => {
    const first = newBlock();
    setBlocks([first]);
    setSelected(0);
    setTitle("");
    setAuthor("");
    setSaved(null);
    focusNext.current = { id: first.id, caret: 0 };
  };

  return (
    <div className="formatted-pdf">
      <Card className="workspace-control-panel py-0">
        <CardContent className="workspace-control-panel-content grid gap-4 p-4">
          <div>
            <h2 className="workspace-active-command">{utility.title}</h2>
            <p className="workspace-panel-copy">{utility.blurb}</p>
          </div>
          <div className="formatted-pdf-layout">
            <section className="formatted-pdf-editor" aria-label="Document">
              <div className="formatted-pdf-toolbar" role="toolbar" aria-label="Formatting">
                <Label htmlFor="formatted-pdf-block-type" className="sr-only">Block type</Label>
                <NativeSelect
                  id="formatted-pdf-block-type"
                  size="sm"
                  value={current.kind}
                  disabled={saving}
                  onChange={(event) => update(blocks.indexOf(current), { kind: event.target.value as BlockKind })}
                >
                  {(Object.keys(kindLabels) as BlockKind[]).map((kind) => <NativeSelectOption key={kind} value={kind}>{kindLabels[kind]}</NativeSelectOption>)}
                </NativeSelect>
              </div>
              <ol ref={list} className="formatted-pdf-blocks" aria-label="Document blocks, in order">
                {blocks.map((block, index) => {
                  const label = `Block ${index + 1}, ${kindLabels[block.kind]}`;
                  return (
                    <li key={block.id} className="formatted-pdf-block" data-kind={block.kind} data-selected={block === current}>
                      <span className="formatted-pdf-gutter" aria-hidden="true">{gutterLabels[block.kind]}</span>
                      <Textarea
                        data-block-id={block.id}
                        aria-label={label}
                        aria-describedby="formatted-pdf-typing-hint"
                        placeholder="Type or paste text here"
                        rows={block.kind === "paragraph" ? 3 : 1}
                        value={block.text}
                        disabled={saving}
                        onFocus={() => setSelected(index)}
                        onChange={(event) => update(index, { text: event.target.value })}
                        onKeyDown={(event) => splitOnEnter(index, event)}
                        onPaste={(event) => splitOnPaste(index, event)}
                      />
                      <div className="formatted-pdf-block-actions">
                        <Button variant="ghost" size="icon" aria-label={`Move block ${index + 1} up`} disabled={saving || index === 0} onClick={() => move(index, -1)}><ArrowUp /></Button>
                        <Button variant="ghost" size="icon" aria-label={`Move block ${index + 1} down`} disabled={saving || index === blocks.length - 1} onClick={() => move(index, 1)}><ArrowDown /></Button>
                        <Button variant="ghost" size="icon" aria-label={`Delete block ${index + 1}`} disabled={saving} onClick={() => remove(index)}><Trash2 /></Button>
                      </div>
                    </li>
                  );
                })}
              </ol>
              <p id="formatted-pdf-typing-hint" className="workspace-note">Enter starts a new paragraph; Shift+Enter adds a line break. Pasted text keeps its paragraphs.</p>
            </section>
            <aside className="formatted-pdf-settings" aria-label="Document settings">
              <fieldset className="workspace-field" disabled={saving}>
                <legend className="workspace-panel-label">Title and author</legend>
                <div className="workspace-field">
                  <Label htmlFor="formatted-pdf-title">Title</Label>
                  <Input id="formatted-pdf-title" aria-describedby="formatted-pdf-details-hint" value={title} onChange={(event) => setTitle(event.target.value)} />
                </div>
                <div className="workspace-field">
                  <Label htmlFor="formatted-pdf-author">Author</Label>
                  <Input id="formatted-pdf-author" aria-describedby="formatted-pdf-details-hint" value={author} onChange={(event) => setAuthor(event.target.value)} />
                </div>
                <p id="formatted-pdf-details-hint" className="workspace-note">Saved in the PDF's details. The title is also the suggested file name.</p>
              </fieldset>
              <dl className="formatted-pdf-defaults">
                <dt className="workspace-panel-label">Page</dt>
                <dd className="workspace-note">A4, portrait, normal margins</dd>
                <dt className="workspace-panel-label">Font</dt>
                <dd className="workspace-note">Liberation Serif, 11 pt</dd>
              </dl>
            </aside>
          </div>
          <footer className="formatted-pdf-footer">
            {empty && <p id="formatted-pdf-blocker" className="workspace-note">Empty document. Add some text to save a PDF.</p>}
            <Button
              variant="default"
              aria-describedby={empty ? "formatted-pdf-blocker" : undefined}
              disabled={saving || empty}
              onClick={() => void save()}
              className="workspace-primary-action"
              {...shortcutTarget("run")}
            >
              Save PDF…
              <ShortcutHint id="run" />
            </Button>
          </footer>
        </CardContent>
      </Card>
      {(saving || saved) && (
        <ResultList
          results={saved ?? []}
          progress={{ completed: saved ? 1 : 0, total: 1 }}
          loading={saving}
          resultVerb={utility.resultVerb}
          resultNoun={utility.resultNoun}
          runningVerb={utility.runningVerb}
          live={null}
          namedByOutput
        />
      )}
      {saved && !saving && <Button variant="outline" className="formatted-pdf-new" onClick={startOver}>New document</Button>}
    </div>
  );
};
