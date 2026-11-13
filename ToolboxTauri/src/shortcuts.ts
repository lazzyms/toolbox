export type Platform = "macos" | "windows";

export const currentPlatform = (): Platform =>
  typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("win") ? "windows" : "macos";

export const isWindows = () => currentPlatform() === "windows";

// Cmd on macOS, Ctrl on Windows. Ctrl-click on macOS opens a context menu, so it never counts.
export const primaryModifierPressed = (event: { metaKey: boolean; ctrlKey: boolean }) => (isWindows() ? event.ctrlKey : event.metaKey);

export type ShortcutId =
  | "find"
  | "settings"
  | "shortcuts"
  | "open"
  | "close"
  | "run"
  | "cancel"
  | "selectAll"
  | "remove"
  | "move"
  | "undo"
  | "redo"
  | "export"
  | "duplicate"
  | "deleteMark";

// Where a shortcut is listened for. "file-list" fires only while focus is in a file list.
// "editor" is handled by the PDF editor. "system" is handled by the operating system and is listed for reference only.
export type ShortcutScope = "window" | "file-list" | "editor" | "system";

export interface Shortcut {
  id: ShortcutId;
  action: string;
  group: "Everywhere" | "Batch tools" | "PDF Editor";
  scope: ShortcutScope;
  // KeyboardEvent.key values, matched without regard to case.
  keys: readonly string[];
  // Cmd on macOS, Ctrl on Windows.
  primary: boolean;
  firesInTextFields: boolean;
  label: Record<Platform, string>;
}

export const shortcuts: readonly Shortcut[] = [
  { id: "find", action: "Find a tool", group: "Everywhere", scope: "window", keys: ["K"], primary: true, firesInTextFields: false, label: { macos: "⌘K", windows: "Ctrl+K" } },
  { id: "settings", action: "Settings", group: "Everywhere", scope: "window", keys: [","], primary: true, firesInTextFields: false, label: { macos: "⌘,", windows: "Ctrl+," } },
  { id: "shortcuts", action: "Keyboard shortcuts", group: "Everywhere", scope: "window", keys: ["/"], primary: true, firesInTextFields: false, label: { macos: "⌘/", windows: "Ctrl+/" } },
  { id: "open", action: "Add files or open a PDF", group: "Everywhere", scope: "window", keys: ["O"], primary: true, firesInTextFields: false, label: { macos: "⌘O", windows: "Ctrl+O" } },
  { id: "close", action: "Close the window", group: "Everywhere", scope: "system", keys: [], primary: false, firesInTextFields: false, label: { macos: "⌘W", windows: "Alt+F4" } },
  { id: "run", action: "Start the run", group: "Batch tools", scope: "window", keys: ["Enter"], primary: true, firesInTextFields: true, label: { macos: "⌘↵", windows: "Ctrl+Enter" } },
  { id: "cancel", action: "Cancel the run", group: "Batch tools", scope: "window", keys: ["Escape"], primary: false, firesInTextFields: false, label: { macos: "Esc", windows: "Esc" } },
  { id: "selectAll", action: "Select all files", group: "Batch tools", scope: "file-list", keys: ["A"], primary: true, firesInTextFields: false, label: { macos: "⌘A", windows: "Ctrl+A" } },
  { id: "remove", action: "Remove selected files", group: "Batch tools", scope: "file-list", keys: ["Delete", "Backspace"], primary: false, firesInTextFields: false, label: { macos: "Delete", windows: "Delete" } },
  { id: "move", action: "Move through the list", group: "Batch tools", scope: "file-list", keys: ["ArrowUp", "ArrowDown"], primary: false, firesInTextFields: false, label: { macos: "↑ ↓", windows: "↑ ↓" } },
  { id: "undo", action: "Undo", group: "PDF Editor", scope: "editor", keys: ["Z"], primary: true, firesInTextFields: false, label: { macos: "⌘Z", windows: "Ctrl+Z" } },
  { id: "redo", action: "Redo", group: "PDF Editor", scope: "editor", keys: ["Y"], primary: true, firesInTextFields: false, label: { macos: "⇧⌘Z", windows: "Ctrl+Y" } },
  { id: "export", action: "Export a copy", group: "PDF Editor", scope: "editor", keys: ["E"], primary: true, firesInTextFields: false, label: { macos: "⌘E", windows: "Ctrl+E" } },
  { id: "duplicate", action: "Duplicate the selected mark", group: "PDF Editor", scope: "editor", keys: ["D"], primary: true, firesInTextFields: false, label: { macos: "⌘D", windows: "Ctrl+D" } },
  { id: "deleteMark", action: "Delete the selected mark", group: "PDF Editor", scope: "editor", keys: ["Delete", "Backspace"], primary: false, firesInTextFields: false, label: { macos: "Delete", windows: "Delete" } },
];

const byId = new Map(shortcuts.map((shortcut) => [shortcut.id, shortcut]));
const shortcut = (id: ShortcutId) => byId.get(id)!;

export const shortcutLabel = (id: ShortcutId) => shortcut(id).label[currentPlatform()];

export const shortcutAria = (id: ShortcutId) => {
  const { keys, primary } = shortcut(id);
  return `${primary ? (isWindows() ? "Control+" : "Meta+") : ""}${keys[0]}`;
};

// Marks the control a window shortcut presses, so the shortcut follows that control's enabled state.
export const shortcutTarget = (id: ShortcutId) => ({ "data-shortcut": id, "aria-keyshortcuts": shortcutAria(id) });

export const pressShortcutTarget = (root: ParentNode | null | undefined, id: ShortcutId) =>
  root?.querySelector<HTMLButtonElement>(`[data-shortcut="${id}"]:not(:disabled)`)?.click();

export const isTextField = (target: EventTarget | null) =>
  target instanceof HTMLElement &&
  (target.isContentEditable || target.matches('input:not([type="checkbox"], [type="radio"], [type="range"]), textarea, select'));

export const matchShortcut = (event: KeyboardEvent, scope: ShortcutScope): ShortcutId | null => {
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.altKey) return null;
  const key = event.key.toLowerCase();
  const primary = event.metaKey || event.ctrlKey;
  const match = shortcuts.find((item) =>
    item.scope === scope && item.primary === primary && item.keys.some((candidate) => candidate.toLowerCase() === key));
  if (!match || (!match.firesInTextFields && isTextField(event.target))) return null;
  return match.id;
};

// Keys inside the PDF Details reports (PdfPrivacy and PdfDetailsNarrow boards). They act on the report that has
// focus, so they are not window shortcuts and are not listed on the Shortcuts board.
export type ReportKeyId = "previousReport" | "nextReport" | "copyReport";

const reportKeys: Record<ReportKeyId, { label: Record<Platform, string>; aria: Record<Platform, string> }> = {
  previousReport: { label: { macos: "⌥↑", windows: "Alt+↑" }, aria: { macos: "Alt+ArrowUp", windows: "Alt+ArrowUp" } },
  nextReport: { label: { macos: "⌥↓", windows: "Alt+↓" }, aria: { macos: "Alt+ArrowDown", windows: "Alt+ArrowDown" } },
  copyReport: { label: { macos: "⇧⌘C", windows: "Ctrl+Shift+C" }, aria: { macos: "Meta+Shift+C", windows: "Control+Shift+C" } },
};

export const reportKeyLabel = (id: ReportKeyId) => reportKeys[id].label[currentPlatform()];
export const reportKeyAria = (id: ReportKeyId) => reportKeys[id].aria[currentPlatform()];

export const matchReportKey = (event: KeyboardEvent): ReportKeyId | null => {
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || isTextField(event.target)) return null;
  const primary = event.metaKey || event.ctrlKey;
  if (event.altKey && !primary && !event.shiftKey) {
    if (event.key === "ArrowUp") return "previousReport";
    if (event.key === "ArrowDown") return "nextReport";
  }
  if (primary && event.shiftKey && !event.altKey && event.key.toLowerCase() === "c") return "copyReport";
  return null;
};

// Keys inside Formatted PDF's block editor (FormattedPdf board). They act on the block field that has focus, so
// they are not window shortcuts and are not listed on the Shortcuts board. They are matched by KeyboardEvent.code
// because Option and Shift change KeyboardEvent.key for digits.
export type FormatKeyId =
  | "bold"
  | "italic"
  | "heading1"
  | "heading2"
  | "heading3"
  | "paragraph"
  | "bulletedList"
  | "numberedList"
  | "pageBreak"
  | "moveUp"
  | "moveDown";

interface FormatKey { code: string; primary: boolean; shift: boolean; alt: boolean; key: string; aria: string }

const formatKeys: Record<FormatKeyId, FormatKey> = {
  bold: { code: "KeyB", primary: true, shift: false, alt: false, key: "B", aria: "B" },
  italic: { code: "KeyI", primary: true, shift: false, alt: false, key: "I", aria: "I" },
  heading1: { code: "Digit1", primary: true, shift: false, alt: true, key: "1", aria: "1" },
  heading2: { code: "Digit2", primary: true, shift: false, alt: true, key: "2", aria: "2" },
  heading3: { code: "Digit3", primary: true, shift: false, alt: true, key: "3", aria: "3" },
  paragraph: { code: "Digit0", primary: true, shift: false, alt: true, key: "0", aria: "0" },
  bulletedList: { code: "Digit8", primary: true, shift: true, alt: false, key: "8", aria: "8" },
  numberedList: { code: "Digit7", primary: true, shift: true, alt: false, key: "7", aria: "7" },
  pageBreak: { code: "Enter", primary: true, shift: true, alt: false, key: "↵", aria: "Enter" },
  moveUp: { code: "ArrowUp", primary: false, shift: true, alt: true, key: "↑", aria: "ArrowUp" },
  moveDown: { code: "ArrowDown", primary: false, shift: true, alt: true, key: "↓", aria: "ArrowDown" },
};

export const formatKeyLabel = (id: FormatKeyId) => {
  const { primary, shift, alt, key } = formatKeys[id];
  if (isWindows()) return [primary && "Ctrl", alt && "Alt", shift && "Shift", key === "↵" ? "Enter" : key].filter(Boolean).join("+");
  return `${alt ? "⌥" : ""}${shift ? "⇧" : ""}${primary ? "⌘" : ""}${key}`;
};

export const formatKeyAria = (id: FormatKeyId) => {
  const { primary, shift, alt, aria } = formatKeys[id];
  return [primary && (isWindows() ? "Control" : "Meta"), alt && "Alt", shift && "Shift", aria].filter(Boolean).join("+");
};

/** The format key an event presses, or null. IME composition never formats; the caller decides which fields listen. */
export const matchFormatKey = (event: KeyboardEvent): FormatKeyId | null => {
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229) return null;
  const primary = primaryModifierPressed(event);
  // The other platform's primary key (Ctrl on macOS, the Windows key on Windows) is never part of a format key.
  if (isWindows() ? event.metaKey : event.ctrlKey) return null;
  const code = event.code === "NumpadEnter" ? "Enter" : event.code;
  const match = (Object.keys(formatKeys) as FormatKeyId[]).find((id) => {
    const key = formatKeys[id];
    return key.code === code && key.primary === primary && key.shift === event.shiftKey && key.alt === event.altKey;
  });
  return match ?? null;
};
