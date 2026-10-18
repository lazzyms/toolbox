import { invoke } from '@tauri-apps/api/core';
import type { OutputLocation } from '../contracts';
import type { SaveDialog } from '../registry/saveSteps';

export interface SaveRequest {
    dialog: SaveDialog;
    folder: string;
    name: string;
    summary: string | null;
}

export type PendingSave = SaveRequest & { id: number; resolve: (path: string | null) => void };

export type SaveName = { ok: true; name: string } | { ok: false; error: string };

const separatorOf = (path: string) => (path.includes('\\') && !path.includes('/') ? '\\' : '/');

const parentFolder = (path: string) => {
    const cut = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
    return cut > 0 ? path.slice(0, cut) : path.slice(0, cut + 1);
};

export const joinPath = (folder: string, name: string) => {
    const separator = separatorOf(folder);
    return folder.endsWith(separator) ? `${folder}${name}` : `${folder}${separator}${name}`;
};

export const proposedSaveName = (inputPath: string, suffix: string, extension: string) => {
    const name = inputPath.split(/[\\/]/).pop() ?? inputPath;
    const dot = name.lastIndexOf('.');
    return `${dot > 0 ? name.slice(0, dot) : name}${suffix}.${extension}`;
};

export const proposedSaveFolder = (firstInput: string, location: OutputLocation) =>
    location === 'alongsideInput' ? parentFolder(firstInput) : location.customFolder;

export const parseSaveName = (raw: string, extension: string): SaveName => {
    const name = raw.trim();
    if (name.length === 0) return { ok: false, error: 'Enter a file name.' };
    if (/[\\/]/.test(name)) return { ok: false, error: 'A file name can’t contain / or \\.' };
    return { ok: true, name: name.toLowerCase().endsWith(`.${extension}`) ? name : `${name}.${extension}` };
};

let pending: PendingSave | null = null;
let lastId = 0;
const listeners = new Set<() => void>();

const setPending = (next: PendingSave | null) => {
    pending = next;
    listeners.forEach((listener) => listener());
};

export const pendingSave = () => pending;

export const subscribeToSaves = (listener: () => void) => {
    listeners.add(listener);
    return () => { listeners.delete(listener); };
};

export const settleSave = (path: string | null) => {
    const current = pending;
    if (!current) return;
    setPending(null);
    current.resolve(path);
};

const askToSave = (request: SaveRequest) => new Promise<string | null>((resolve) => {
    settleSave(null);
    lastId += 1;
    setPending({ ...request, id: lastId, resolve });
});

export const chooseManyToOneSavePath = (dialog: SaveDialog, files: string[], location: OutputLocation, suffix: string, summary: string) =>
    askToSave({ dialog, folder: proposedSaveFolder(files[0], location), name: proposedSaveName(files[0], suffix, dialog.extension), summary });

export const chooseCreatedDocumentSavePath = async (dialog: SaveDialog, proposedName: string, summary: string | null = null) =>
    askToSave({ dialog, folder: await invoke<string>('get_last_save_folder'), name: proposedName, summary });
