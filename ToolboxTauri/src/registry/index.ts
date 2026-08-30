export type ToolCategory = 'PDF' | 'Images';

export interface Utility {
    id: string;
    title: string;
    shortTitle: string;
    blurb: string;
    symbol: string;
    tint: string;
    category: ToolCategory;
}

export const UtilityRegistry: Utility[] = [
    {
        id: 'pdf-unlock',
        title: 'Remove PDF Password',
        shortTitle: 'Unlock PDF',
        blurb: 'Save an unlocked copy of a PDF you know the password for.',
        symbol: 'lock-open',
        tint: '#f97316',
        category: 'PDF',
    },
    {
        id: 'pdf-protect',
        title: 'Protect PDF',
        shortTitle: 'Protect',
        blurb: 'Add a password so only you can open it.',
        symbol: 'lock',
        tint: '#ef4444',
        category: 'PDF',
    },
    {
        id: 'image-compress',
        title: 'Compress Images',
        shortTitle: 'Compress',
        blurb: 'Shrink files losslessly, or trade quality for size.',
        symbol: 'file-download',
        tint: '#22c55e',
        category: 'Images',
    },
    {
        id: 'image-convert',
        title: 'Convert Image Format',
        shortTitle: 'Convert',
        blurb: 'HEIC to PNG, JPEG and back — batch friendly.',
        symbol: 'arrows-exchange',
        tint: '#3b82f6',
        category: 'Images',
    },
];
