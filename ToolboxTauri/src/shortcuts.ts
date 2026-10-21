export type Platform = "macos" | "windows";

export const currentPlatform = (): Platform =>
  typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("win") ? "windows" : "macos";

export const isWindows = () => currentPlatform() === "windows";

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
  | "move";

// Where a shortcut is listened for. "file-list" fires only while focus is in a file list.
// "system" is handled by the operating system and is listed for reference only.
export type ShortcutScope = "window" | "file-list" | "system";

export interface Shortcut {
  id: ShortcutId;
  action: string;
  group: "Everywhere" | "Batch tools";
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

const isTextField = (target: EventTarget | null) =>
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
