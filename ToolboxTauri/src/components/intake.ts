import type { ToolDefinition } from '../contracts';

export type PathKind = 'file' | 'folder' | 'unreadable';

export type InspectedPath = { path: string; kind: PathKind };

export type IntakeEntry = { key: string; path: string };

export type NotAddedReason = 'notAccepted' | 'folder' | 'duplicate' | 'unreadable';

export type NotAdded = { path: string; reason: NotAddedReason; message: string };

export type IntakePolicy = {
    label: string;
    acceptedExtensions: string[];
    allowsRepeats: boolean;
    notAcceptedMessage?: string;
};

export type IntakeCopy = {
    notAccepted: string;
    unsupported: (path: string) => string;
    willFail?: (path: string) => string | null;
    flagLabel?: (path: string) => string;
    flaggedNotice: (count: number) => string;
    countHint: (flagged: number, total: number) => string;
};

// A wontRun file stays out of the run. A willFail file is sent and fails on its own result row.
export type EntryFlag = { kind: 'wontRun' | 'willFail'; message: string; label?: string };

export type EntryStatus = {
    entry: IntakeEntry;
    occurrence: number;
    flag: EntryFlag | null;
};

const pathKinds: ReadonlySet<string> = new Set<PathKind>(['file', 'folder', 'unreadable']);

export const parseInspection = (raw: unknown, paths: string[]): InspectedPath[] => {
    const kinds = new Map<string, PathKind>();
    if (Array.isArray(raw)) {
        for (const item of raw) {
            if (typeof item?.path === 'string' && pathKinds.has(item?.kind)) kinds.set(item.path, item.kind);
        }
    }
    return paths.map((path) => ({ path, kind: kinds.get(path) ?? 'file' }));
};

export const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

export const parentFolder = (path: string) => {
    const index = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    return index > 0 ? path.slice(0, index) : '';
};

export const extensionOf = (path: string) => {
    const name = fileName(path);
    const dot = name.lastIndexOf('.');
    return dot > 0 ? name.slice(dot).toLowerCase() : '';
};

const accepts = (extensions: readonly string[], path: string) =>
    extensions.some((extension) => extension.toLowerCase() === extensionOf(path));

const extensionList = (extensions: readonly string[]) => `${extensions.join(', ')} files`;

export const allowsRepeats = (tool: ToolDefinition) => tool.capability.inputCardinality === 'ordered';

export const intakePolicyFor = (label: string, tools: ToolDefinition[], active: ToolDefinition): IntakePolicy => ({
    label,
    acceptedExtensions: [...new Set(tools.flatMap((tool) => tool.capability.acceptedExtensions.map((extension) => extension.toLowerCase())))],
    allowsRepeats: allowsRepeats(active),
});

export const notAddedMessage = (reason: NotAddedReason, policy: IntakePolicy) => {
    switch (reason) {
        case 'notAccepted': return policy.notAcceptedMessage ?? `${policy.label} only takes ${extensionList(policy.acceptedExtensions)}.`;
        case 'folder': return "Folders aren't added. Open the folder and drop the files inside it.";
        case 'duplicate': return 'Already in the list, so it was added once.';
        case 'unreadable': return "Couldn't open it. It may be in use by another app, or you don't have permission to read it. The other files are unaffected.";
    }
};

export const classifyIntake = (existing: string[], incoming: InspectedPath[], policy: IntakePolicy) => {
    const present = new Set(existing);
    const added: string[] = [];
    const notAdded: NotAdded[] = [];
    const refuse = (path: string, reason: NotAddedReason) =>
        notAdded.push({ path, reason, message: notAddedMessage(reason, policy) });
    for (const { path, kind } of incoming) {
        if (kind === 'folder') refuse(path, 'folder');
        else if (kind === 'unreadable') refuse(path, 'unreadable');
        else if (!accepts(policy.acceptedExtensions, path)) refuse(path, 'notAccepted');
        else if (present.has(path) && !policy.allowsRepeats) refuse(path, 'duplicate');
        else {
            present.add(path);
            added.push(path);
        }
    }
    return { added, notAdded };
};

export const entryStatuses = (
    entries: IntakeEntry[],
    tool: ToolDefinition,
    copy: Partial<Pick<IntakeCopy, 'unsupported' | 'willFail' | 'flagLabel'>> = {},
): EntryStatus[] => {
    const unsupported = copy.unsupported ?? (() => `${tool.title} only takes ${extensionList(tool.capability.acceptedExtensions)}.`);
    const seen = new Map<string, number>();
    const repeats = allowsRepeats(tool);
    return entries.map((entry) => {
        const occurrence = (seen.get(entry.path) ?? 0) + 1;
        seen.set(entry.path, occurrence);
        const willFail = copy.willFail?.(entry.path) ?? null;
        const flag: EntryFlag | null = !accepts(tool.capability.acceptedExtensions, entry.path)
            ? { kind: 'wontRun', message: unsupported(entry.path) }
            : occurrence > 1 && !repeats
                ? { kind: 'wontRun', message: `${tool.title} runs each file once.` }
                : willFail !== null
                    ? { kind: 'willFail', message: willFail, label: copy.flagLabel?.(entry.path) }
                    : null;
        return { entry, occurrence, flag };
    });
};

export const moveEntry = (entries: IntakeEntry[], key: string, toIndex: number) => {
    const from = entries.findIndex((entry) => entry.key === key);
    if (from < 0 || toIndex < 0 || toIndex >= entries.length || from === toIndex) return entries;
    const next = [...entries];
    const [moved] = next.splice(from, 1);
    next.splice(toIndex, 0, moved);
    return next;
};

export const rangeKeys = (entries: IntakeEntry[], anchor: string, target: string) => {
    const a = entries.findIndex((entry) => entry.key === anchor);
    const b = entries.findIndex((entry) => entry.key === target);
    if (a < 0 || b < 0) return [target];
    return entries.slice(Math.min(a, b), Math.max(a, b) + 1).map((entry) => entry.key);
};

/** Facts read from each listed file. A fact that couldn't be read is null and the row leaves it out. */
export type EntryFacts = { pageCount: number | null; modifiedMs: number | null; sizeBytes: number | null };

export type EntrySort = 'added' | 'nameAsc' | 'nameDesc' | 'modifiedAsc' | 'modifiedDesc';

export type EntrySortOption = { value: EntrySort; label: string };

/** Every sort an ordered list can offer. A tool may offer a subset, with its own labels. */
export const entrySortOptions: readonly EntrySortOption[] = [
    { value: 'added', label: 'Order added' },
    { value: 'nameAsc', label: 'Name, A to Z' },
    { value: 'nameDesc', label: 'Name, Z to A' },
    { value: 'modifiedAsc', label: 'Date modified, oldest first' },
    { value: 'modifiedDesc', label: 'Date modified, newest first' },
];

export const sortNeedsFacts = (sort: EntrySort) => sort === 'modifiedAsc' || sort === 'modifiedDesc';

const names = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

/** Returns the entries in the chosen order. Ties and files without a date keep their current relative order, dateless files last. */
export const sortEntries = (entries: IntakeEntry[], sort: EntrySort, facts: (path: string) => EntryFacts | undefined, addedOrder: (entry: IntakeEntry) => number) => {
    const modified = (entry: IntakeEntry) => facts(entry.path)?.modifiedMs ?? null;
    const byModified = (direction: 1 | -1) => (a: IntakeEntry, b: IntakeEntry) => {
        const left = modified(a);
        const right = modified(b);
        if (left === null || right === null) return Number(left === null) - Number(right === null);
        return direction * (left - right);
    };
    const compare: Record<EntrySort, (a: IntakeEntry, b: IntakeEntry) => number> = {
        added: (a, b) => addedOrder(a) - addedOrder(b),
        nameAsc: (a, b) => names.compare(fileName(a.path), fileName(b.path)),
        nameDesc: (a, b) => names.compare(fileName(b.path), fileName(a.path)),
        modifiedAsc: byModified(1),
        modifiedDesc: byModified(-1),
    };
    return [...entries].sort(compare[sort]);
};

export const pageCountLabel = (count: number) => count === 1 ? '1 page' : `${count} pages`;

export const fileSizeLabel = (bytes: number) => {
    if (bytes < 1000) return `${bytes} bytes`;
    if (bytes < 1000 * 1000) return `${Math.round(bytes / 1000)} KB`;
    if (bytes < 1000 * 1000 * 1000) return `${(bytes / (1000 * 1000)).toFixed(1)} MB`;
    return `${(bytes / (1000 * 1000 * 1000)).toFixed(1)} GB`;
};

export const modifiedLabel = (modifiedMs: number) =>
    new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', year: 'numeric' }).format(new Date(modifiedMs));

export const parseEntryFacts = (value: unknown): Map<string, EntryFacts> => {
    const facts = new Map<string, EntryFacts>();
    if (!Array.isArray(value)) return facts;
    const number = (field: unknown) => typeof field === 'number' && Number.isFinite(field) ? field : null;
    for (const item of value) {
        if (!item || typeof item !== 'object' || typeof (item as { path?: unknown }).path !== 'string') continue;
        const { path, pageCount, modifiedMs, sizeBytes } = item as Record<string, unknown>;
        facts.set(path as string, { pageCount: number(pageCount), modifiedMs: number(modifiedMs), sizeBytes: number(sizeBytes) });
    }
    return facts;
};
