import { formatBytes } from '../shared/fileSize';

/** Markup pasted into the list. It has no file, so it stands in the list under a made-up path. */
export interface PastedHtml {
    name: string;
    markup: string;
    bytes: number;
    remote: string | null;
    /** Where the user chose to save it for the latest run. */
    outputPath: string | null;
}

const PREFIX = 'pasted-html:';
const items = new Map<string, PastedHtml>();
let lastId = 0;

const isRemote = (address: string | null) => address !== null && /^(https?:)?\/\//i.test(address.trim());

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

/** What the backend will leave out, read from the markup without running or loading any of it. */
export const remoteNote = (markup: string): string | null => {
    const page = new DOMParser().parseFromString(markup, 'text/html');
    const count = (selector: string, attribute: string) =>
        [...page.querySelectorAll(selector)].filter((element) => isRemote(element.getAttribute(attribute))).length;
    const images = count('img[src]', 'src');
    const stylesheets = count('link[rel~="stylesheet" i][href]', 'href');
    const embeds = count('iframe[src], frame[src], embed[src], video[src], audio[src]', 'src') + count('object[data]', 'data');
    const parts = [
        images > 0 ? plural(images, 'remote image', 'remote images') : null,
        stylesheets > 0 ? plural(stylesheets, 'remote stylesheet', 'remote stylesheets') : null,
        embeds > 0 ? plural(embeds, 'remote embedded item', 'remote embedded items') : null,
    ].filter((part): part is string => part !== null);
    if (parts.length === 0) return null;
    const listed = parts.length === 1 ? parts[0] : `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`;
    return `${listed} will be left out`;
};

/** The page title, else its first heading, as the suggested name in the list. */
export const pastedHtmlTitle = (markup: string) => {
    const page = new DOMParser().parseFromString(markup, 'text/html');
    const title = page.title.trim() || page.querySelector('h1')?.textContent?.trim() || '';
    return title.replace(/\s+/g, ' ').slice(0, 120);
};

export const pastedSizeNote = (markup: string) => {
    const bytes = new TextEncoder().encode(markup).length;
    const remote = remoteNote(markup);
    return `${formatBytes(bytes)}.${remote ? ` ${remote}.` : ''}`;
};

export const addPastedHtml = (name: string, markup: string) => {
    lastId += 1;
    const safeName = name.trim().replace(/[\\/]/g, '-') || 'Pasted page';
    const path = `${PREFIX}${lastId}/${safeName}.html`;
    items.set(path, { name: safeName, markup, bytes: new TextEncoder().encode(markup).length, remote: remoteNote(markup), outputPath: null });
    return path;
};

export const isPastedHtml = (path: string) => path.startsWith(PREFIX);

export const pastedHtml = (path: string) => items.get(path);

export const choosePastedOutput = (path: string, outputPath: string) => {
    const item = items.get(path);
    if (item) item.outputPath = outputPath;
};

export const pastedRowDetail = (item: PastedHtml) => `Pasted HTML, ${formatBytes(item.bytes)}${item.remote ? `. ${item.remote}` : ''}`;
