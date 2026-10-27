import { FolderIcon } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import type { OutputLocation, ToolCommand } from '../contracts';
import { shortenHomePath, useHomeDir } from '../homePath';

export type FolderCheck = 'ready' | 'missing' | 'unwritable';

export type BlockedDestination = { folder: string; check: Exclude<FolderCheck, 'ready'> };

const ALONGSIDE_LABEL = 'Next to each original';

const describe = (location: OutputLocation, home: string | null) =>
    location === 'alongsideInput' ? ALONGSIDE_LABEL : shortenHomePath(location.customFolder, home);

const folderName = (folder: string) => folder.split(/[\\/]/).filter(Boolean).pop() ?? folder;

export const sameLocation = (a: OutputLocation, b: OutputLocation) =>
    a === 'alongsideInput' || b === 'alongsideInput' ? a === b : a.customFolder === b.customFolder;

// Commands that always write a different file type than they read, as named by the native output code.
const FIXED_OUTPUT_EXTENSIONS: Partial<Record<ToolCommand, string>> = {
    pdf_to_text: 'txt',
    ocr_pdf: 'txt',
    extract_pdf_images: 'jpg',
    remove_image_background: 'png',
    blur_faces: 'png',
    generate_icon_set: 'png',
    extract_gif_frames: 'png',
};

export const exampleOutputName = (inputPath: string, command: ToolCommand, suffix: string, extension = FIXED_OUTPUT_EXTENSIONS[command]) => {
    const name = inputPath.split(/[\\/]/).pop() ?? inputPath;
    const dot = name.lastIndexOf('.');
    const stem = dot > 0 ? name.slice(0, dot) : name;
    const outputExtension = extension ?? (dot > 0 ? name.slice(dot + 1) : '');
    return outputExtension ? `${stem}${suffix}.${outputExtension}` : `${stem}${suffix}`;
};

interface RunDestinationProps {
    location: OutputLocation;
    defaultLocation: OutputLocation;
    overridden: boolean;
    toolTitle: string;
    exampleName: string | null;
    savesOneFile?: boolean;
    onChange: () => void;
    onReset: () => void;
}

export const RunDestination = ({ location, defaultLocation, overridden, toolTitle, exampleName, savesOneFile = false, onChange, onReset }: RunDestinationProps) => {
    const home = useHomeDir();
    return (
    <section aria-label="Destination" className="run-destination">
        <div className="run-destination-heading">
            <span className="run-destination-label">Save to</span>
            {overridden && <Badge variant="outline">This run only</Badge>}
        </div>
        {!savesOneFile && <>
        <div className="run-destination-path">
            <FolderIcon aria-hidden="true" />
            <span data-testid="run-destination-path">{describe(location, home)}</span>
        </div>
        <div className="run-destination-actions">
            {overridden ? <>
                <Button variant="link" size="sm" onClick={onChange}>Change</Button>
                <Button variant="link" size="sm" onClick={onReset}>Reset to default</Button>
            </> : <Button variant="link" size="sm" onClick={onChange}>Change for this run</Button>}
        </div>
        {overridden && (
            <p className="run-destination-hint">
                Your default is {defaultLocation === 'alongsideInput' ? 'next to each original' : shortenHomePath(defaultLocation.customFolder, home)}. This change ends when you leave {toolTitle}.
            </p>
        )}
        </>}
        {savesOneFile && (
            <p className="run-destination-hint">You choose a name and folder when you start, because this makes one file.</p>
        )}
        {exampleName && (
            <div className="run-destination-example">
                <span className="run-destination-hint">Example</span>
                <span data-testid="run-destination-example">{exampleName}</span>
            </div>
        )}
        {exampleName && !overridden && (
            <p className="run-destination-hint">An existing name gets -1, -2 and so on. Nothing is replaced.</p>
        )}
    </section>
    );
};

interface BlockedDestinationDialogProps {
    blocked: BlockedDestination | null;
    onCancel: () => void;
    onSaveAlongside: () => void;
    onChooseAnother: () => void;
}

export const BlockedDestinationDialog = ({ blocked, onCancel, onSaveAlongside, onChooseAnother }: BlockedDestinationDialogProps) => (
    <Dialog open={blocked !== null} onOpenChange={(open) => { if (!open) onCancel(); }}>
        {blocked && (
            <DialogContent role="alertdialog" showCloseButton={false} className="sm:max-w-[440px]">
                <DialogHeader>
                    <DialogTitle>
                        {blocked.check === 'missing'
                            ? `The folder ${folderName(blocked.folder)} is missing`
                            : `The folder ${folderName(blocked.folder)} can't be written to`}
                    </DialogTitle>
                    <DialogDescription className="[overflow-wrap:anywhere]">
                        {blocked.check === 'missing'
                            ? `Toolbox can't find ${blocked.folder}. It may have been moved or renamed, or it's on a drive that isn't connected. Nothing has run yet.`
                            : `Toolbox can't save files in ${blocked.folder}. Check its permissions or choose another folder. Nothing has run yet.`}
                    </DialogDescription>
                </DialogHeader>
                <DialogFooter>
                    <Button variant="ghost" onClick={onCancel}>Cancel</Button>
                    <Button variant="outline" onClick={onSaveAlongside}>Save next to originals</Button>
                    <Button onClick={onChooseAnother}>Choose another folder</Button>
                </DialogFooter>
            </DialogContent>
        )}
    </Dialog>
);
