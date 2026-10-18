import { useState, useSyncExternalStore } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderIcon } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { shortenHomePath, useHomeDir } from '../homePath';
import { joinPath, parseSaveName, pendingSave, settleSave, subscribeToSaves, type PendingSave } from './saveStep';

const SaveForm = ({ request }: { request: PendingSave }) => {
    const home = useHomeDir();
    const [name, setName] = useState(request.name);
    const [folder, setFolder] = useState(request.folder);
    const [folderError, setFolderError] = useState<string | null>(null);
    const parsed = parseSaveName(name, request.dialog.extension);

    const changeFolder = async () => {
        try {
            const picked = await open({ directory: true, multiple: false, defaultPath: folder });
            if (typeof picked === 'string' && picked.length > 0) setFolder(picked);
            setFolderError(null);
        } catch (error) {
            setFolderError(String(error));
        }
    };

    return (
        <form
            className="grid gap-4"
            onSubmit={(event) => {
                event.preventDefault();
                if (parsed.ok) settleSave(joinPath(folder, parsed.name));
            }}
        >
            <DialogHeader>
                <DialogTitle>{request.dialog.title}</DialogTitle>
            </DialogHeader>
            <div className="grid gap-2">
                <Label htmlFor="save-file-name">File name</Label>
                <Input
                    id="save-file-name"
                    value={name}
                    autoFocus
                    spellCheck={false}
                    aria-invalid={!parsed.ok}
                    aria-describedby={parsed.ok ? undefined : 'save-file-name-error'}
                    onChange={(event) => setName(event.target.value)}
                />
                {!parsed.ok && <p id="save-file-name-error" role="alert" className="text-sm text-destructive">{parsed.error}</p>}
            </div>
            <div className="grid gap-2">
                <span id="save-folder-label" className="text-sm leading-none font-medium">Save to</span>
                <div className="flex items-center gap-2">
                    <span aria-labelledby="save-folder-label" data-testid="save-folder" className="flex min-w-0 flex-1 items-center gap-2 text-sm [overflow-wrap:anywhere]">
                        <FolderIcon aria-hidden="true" className="size-4 shrink-0" />
                        {shortenHomePath(folder, home)}
                    </span>
                    <Button type="button" variant="outline" size="sm" onClick={() => void changeFolder()}>Change…</Button>
                </div>
                {folderError && <p role="alert" className="text-sm text-destructive">{folderError}</p>}
            </div>
            {request.summary && <DialogDescription>{request.summary}</DialogDescription>}
            <DialogFooter>
                <Button type="button" variant="outline" onClick={() => settleSave(null)}>Cancel</Button>
                <Button type="submit" disabled={!parsed.ok}>Save</Button>
            </DialogFooter>
        </form>
    );
};

export const SaveDialogHost = () => {
    const request = useSyncExternalStore(subscribeToSaves, pendingSave);
    return (
        <Dialog open={request !== null} onOpenChange={(isOpen) => { if (!isOpen) settleSave(null); }}>
            {request && (
                <DialogContent showCloseButton={false} {...(request.summary ? {} : { 'aria-describedby': undefined })} className="sm:max-w-[500px]">
                    <SaveForm key={request.id} request={request} />
                </DialogContent>
            )}
        </Dialog>
    );
};
