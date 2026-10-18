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
};

export type EntryStatus = {
    entry: IntakeEntry;
    occurrence: number;
    wontRun: string | null;
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
        case 'notAccepted': return `${policy.label} only takes ${extensionList(policy.acceptedExtensions)}.`;
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

export const entryStatuses = (entries: IntakeEntry[], tool: ToolDefinition): EntryStatus[] => {
    const seen = new Map<string, number>();
    const repeats = allowsRepeats(tool);
    return entries.map((entry) => {
        const occurrence = (seen.get(entry.path) ?? 0) + 1;
        seen.set(entry.path, occurrence);
        const wontRun = !accepts(tool.capability.acceptedExtensions, entry.path)
            ? `${tool.title} only takes ${extensionList(tool.capability.acceptedExtensions)}.`
            : occurrence > 1 && !repeats
                ? `${tool.title} runs each file once.`
                : null;
        return { entry, occurrence, wontRun };
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
