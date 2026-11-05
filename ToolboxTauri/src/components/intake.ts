import type { ToolDefinition } from '../contracts';

export type PathKind = 'file' | 'folder' | 'unreadable';

/** `animated` is true when the native side found an animated PNG or WebP. */
export type InspectedPath = { path: string; kind: PathKind; animated?: boolean };

export type IntakeEntry = { key: string; path: string; animated?: boolean };

export type NotAddedReason = 'notAccepted' | 'folder' | 'duplicate' | 'unreadable';

export type NotAdded = { path: string; reason: NotAddedReason; message: string };

export type IntakePolicy = {
    label: string;
    acceptedExtensions: string[];
    allowsRepeats: boolean;
    notAcceptedMessage?: string;
    /** Keeps files of any type in the list, where the active tool flags the ones it can't take. */
    keepsUnsupported?: boolean;
};

export type IntakeCopy = {
    notAccepted: string;
    /** Keeps unsupported files in the list, flagged, instead of listing them as not added. */
    keepsUnsupported?: boolean;
    unsupported: (path: string) => string;
    /** Why a file of one of the tool's skipped extensions will be skipped. */
    willSkip?: (path: string) => string;
    /** Why an animated image won't run, for tools that take only still images. */
    animated?: (path: string) => string;
    willFail?: (path: string) => string | null;
    flaggedNotice: (count: number, skipped: number) => string;
    countHint: (flagged: number, total: number) => string;
};

// A wontRun file stays out of the run. A willFail file is sent and fails on its own result row.
// A willSkip file is sent and comes back Skipped, with a pointer to the tool that takes it.
export type EntryFlag = { kind: 'wontRun' | 'willFail' | 'willSkip'; message: string };

export type EntryStatus = {
    entry: IntakeEntry;
    occurrence: number;
    flag: EntryFlag | null;
};

const pathKinds: ReadonlySet<string> = new Set<PathKind>(['file', 'folder', 'unreadable']);

export const parseInspection = (raw: unknown, paths: string[]): InspectedPath[] => {
    const kinds = new Map<string, PathKind>();
    const animated = new Set<string>();
    if (Array.isArray(raw)) {
        for (const item of raw) {
            if (typeof item?.path === 'string' && pathKinds.has(item?.kind)) kinds.set(item.path, item.kind);
            if (typeof item?.path === 'string' && item?.animated === true) animated.add(item.path);
        }
    }
    return paths.map((path) => ({ path, kind: kinds.get(path) ?? 'file', animated: animated.has(path) }));
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
    acceptedExtensions: [...new Set(tools.flatMap((tool) => [...tool.capability.acceptedExtensions, ...tool.capability.skippedExtensions].map((extension) => extension.toLowerCase())))],
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
    const added: InspectedPath[] = [];
    const notAdded: NotAdded[] = [];
    const refuse = (path: string, reason: NotAddedReason) =>
        notAdded.push({ path, reason, message: notAddedMessage(reason, policy) });
    for (const item of incoming) {
        const { path, kind } = item;
        if (kind === 'folder') refuse(path, 'folder');
        else if (kind === 'unreadable') refuse(path, 'unreadable');
        else if (!policy.keepsUnsupported && !accepts(policy.acceptedExtensions, path)) refuse(path, 'notAccepted');
        else if (present.has(path) && !policy.allowsRepeats) refuse(path, 'duplicate');
        else {
            present.add(path);
            added.push(item);
        }
    }
    return { added, notAdded };
};

export const entryStatuses = (
    entries: IntakeEntry[],
    tool: ToolDefinition,
    copy: Partial<Pick<IntakeCopy, 'unsupported' | 'willSkip' | 'animated' | 'willFail'>> = {},
): EntryStatus[] => {
    const unsupported = copy.unsupported ?? (() => `${tool.title} only takes ${extensionList(tool.capability.acceptedExtensions)}.`);
    const seen = new Map<string, number>();
    const repeats = allowsRepeats(tool);
    return entries.map((entry) => {
        const occurrence = (seen.get(entry.path) ?? 0) + 1;
        seen.set(entry.path, occurrence);
        const willFail = copy.willFail?.(entry.path) ?? null;
        const skipped = accepts(tool.capability.skippedExtensions, entry.path);
        const flag: EntryFlag | null = !skipped && !accepts(tool.capability.acceptedExtensions, entry.path)
            ? { kind: 'wontRun', message: unsupported(entry.path) }
            : occurrence > 1 && !repeats
                ? { kind: 'wontRun', message: `${tool.title} runs each file once.` }
                : skipped
                    ? { kind: 'willSkip', message: copy.willSkip?.(entry.path) ?? `${tool.title} skips ${extensionOf(entry.path)} files.` }
                    : entry.animated && copy.animated
                        ? { kind: 'wontRun', message: copy.animated(entry.path) }
                        : willFail !== null
                            ? { kind: 'willFail', message: willFail }
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
