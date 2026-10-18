import { useEffect, useId, useState } from "react";
import { CircleX, Eye, EyeOff, KeyRound } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { OutputLocation, PasswordRequest, PasswordSet } from "../contracts";
import { fileName } from "./resultRow";

export type PasswordMode = "lock" | "unlock";

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;

// Only the overrides of files in this request travel, so a retry or a smaller run never carries other files' passwords.
export const passwordRequest = (paths: string[], passwords: PasswordSet, outputLocation: OutputLocation): PasswordRequest => ({
    paths,
    password: passwords.batch,
    passwordOverrides: Object.fromEntries(paths.flatMap((path) => path in passwords.overrides ? [[path, passwords.overrides[path]]] : [])),
    outputLocation,
});

export const usePasswordSet = () => {
    const [batch, setBatch] = useState("");
    const [confirm, setConfirm] = useState("");
    const [overrides, setOverrides] = useState<Record<string, string>>({});
    const [editing, setEditing] = useState<string | null>(null);
    const [cleared, setCleared] = useState<{ path: string; password: string } | null>(null);

    const clear = () => {
        setBatch("");
        setConfirm("");
        setOverrides({});
        setEditing(null);
        setCleared(null);
    };

    // Closing a file forgets its password. Closing every file forgets the batch password too.
    const keepOnly = (paths: readonly string[]) => {
        if (paths.length === 0) {
            clear();
            return;
        }
        const open = new Set(paths);
        setOverrides((current) => {
            const kept = Object.entries(current).filter(([path]) => open.has(path));
            return kept.length === Object.keys(current).length ? current : Object.fromEntries(kept);
        });
        setEditing((current) => current !== null && open.has(current) ? current : null);
        setCleared((current) => current !== null && open.has(current.path) ? current : null);
    };

    const saveOverride = (path: string, password: string) => {
        setOverrides((current) => ({ ...current, [path]: password }));
        setEditing(null);
        setCleared(null);
    };

    const restoreBatch = (path: string) => {
        const password = overrides[path];
        if (password === undefined) return;
        setOverrides(({ [path]: _removed, ...rest }) => rest);
        setCleared({ path, password });
    };

    const undoClear = () => {
        if (!cleared) return;
        setOverrides((current) => ({ ...current, [cleared.path]: cleared.password }));
        setCleared(null);
    };

    const set: PasswordSet = { batch, overrides };
    return { set, batch, setBatch, confirm, setConfirm, overrides, editing, setEditing, cleared, clear, keepOnly, saveOverride, restoreBatch, undoClear };
};

export type PasswordState = ReturnType<typeof usePasswordSet>;

const confirmMismatch = (mode: PasswordMode, password: string, confirm: string) => mode === "lock" && confirm.length > 0 && confirm !== password;

// Lock asks for the new password twice. Unlock checks against the file, so one entry is enough.
const confirmed = (mode: PasswordMode, password: string, confirm: string) => mode === "unlock" || confirm === password;

export const passwordReadiness = (mode: PasswordMode, passwords: PasswordState, files: readonly string[]) => {
    const missing = files.filter((path) => !(path in passwords.overrides)).length;
    const batchNeeded = missing > 0;
    if (batchNeeded && passwords.batch.length === 0) return { ready: false, reason: `${plural(missing, "file has", "files have")} no password yet` };
    if (batchNeeded && !confirmed(mode, passwords.batch, passwords.confirm)) return { ready: false, reason: "Passwords don't match" };
    return { ready: true, reason: null };
};

const RevealableInput = ({ id, label, value, onChange, visible, onToggle, invalid, describedBy, autoFocus, placeholder }: {
    id: string;
    label: string;
    value: string;
    onChange: (value: string) => void;
    visible: boolean;
    onToggle?: () => void;
    invalid?: boolean;
    describedBy?: string;
    autoFocus?: boolean;
    placeholder?: string;
}) => (
    <div className="workspace-field">
        <Label htmlFor={id}>{label}</Label>
        <div className={onToggle ? "workspace-password-field" : undefined}>
            <Input
                id={id}
                type={visible ? "text" : "password"}
                value={value}
                onChange={(event) => onChange(event.target.value)}
                placeholder={placeholder}
                autoComplete="off"
                spellCheck={false}
                aria-invalid={invalid || undefined}
                aria-describedby={describedBy}
                autoFocus={autoFocus}
            />
            {onToggle && (
                <Button variant="outline" size="icon" aria-label={`${visible ? "Hide" : "Show"} ${label.toLowerCase()}`} aria-pressed={visible} onClick={onToggle}>
                    {visible ? <EyeOff aria-hidden="true" /> : <Eye aria-hidden="true" />}
                </Button>
            )}
        </div>
    </div>
);

export const BatchPasswordFields = ({ mode, passwords, files }: { mode: PasswordMode; passwords: PasswordState; files: readonly string[] }) => {
    const [visible, setVisible] = useState(false);
    const [confirmVisible, setConfirmVisible] = useState(false);
    const id = useId();
    const mismatch = confirmMismatch(mode, passwords.batch, passwords.confirm);
    const own = files.filter((path) => path in passwords.overrides).length;
    const batchCount = files.length - own;
    const usage = files.length === 0
        ? null
        : own === 0
            ? `Used for all ${plural(files.length, "file", "files")}.`
            : `Used for ${plural(batchCount, "file", "files")}. ${plural(own, "file has its", "files have their")} own password${own === 1 ? "" : "s"}.`;
    const cleared = passwords.cleared;

    return (
        <div className="password-options">
            <RevealableInput
                id={`${id}-batch`}
                label={mode === "lock" ? "New password for all files" : "Current password for all files"}
                value={passwords.batch}
                onChange={passwords.setBatch}
                visible={visible}
                onToggle={() => setVisible((current) => !current)}
                placeholder="Enter password"
            />
            {mode === "lock" && (
                <RevealableInput
                    id={`${id}-confirm`}
                    label="Confirm new password"
                    value={passwords.confirm}
                    onChange={passwords.setConfirm}
                    visible={confirmVisible}
                    onToggle={() => setConfirmVisible((current) => !current)}
                    invalid={mismatch}
                    describedBy={mismatch ? `${id}-mismatch` : undefined}
                    placeholder="Type it again"
                />
            )}
            {mismatch && <p id={`${id}-mismatch`} className="password-error" role="alert"><CircleX aria-hidden="true" />Passwords don't match. Type the new password again.</p>}
            {usage && <p className="workspace-note" data-testid="password-usage">{usage}</p>}
            {cleared && (
                <p className="password-cleared" role="status">
                    {fileName(cleared.path)} uses the batch password again.
                    <Button variant="ghost" size="xs" onClick={passwords.undoClear}>Undo</Button>
                </p>
            )}
            <p className="workspace-note">Passwords are never saved and are cleared when you switch mode or leave.</p>
        </div>
    );
};

const OwnPasswordEditor = ({ mode, name, initial, onSave, onCancel }: {
    mode: PasswordMode;
    name: string;
    initial: string;
    onSave: (password: string) => void;
    onCancel: () => void;
}) => {
    const [password, setPassword] = useState(initial);
    const [confirm, setConfirm] = useState(initial);
    const id = useId();
    const mismatch = confirmMismatch(mode, password, confirm);
    const canSave = password.length > 0 && confirmed(mode, password, confirm);
    return (
        <div
            className="password-inline"
            role="group"
            aria-label={`Own password for ${name}`}
            onKeyDown={(event) => {
                event.stopPropagation();
                if (event.key === "Escape") onCancel();
                if (event.key === "Enter" && canSave) onSave(password);
            }}
        >
            <div className="password-inline-fields">
                <RevealableInput id={`${id}-own`} label={mode === "lock" ? "Own new password" : "Own current password"} value={password} onChange={setPassword} visible={false} autoFocus />
                {mode === "lock" && <RevealableInput id={`${id}-own-confirm`} label="Confirm" value={confirm} onChange={setConfirm} visible={false} invalid={mismatch} />}
                <span className="password-inline-actions">
                    <Button size="sm" disabled={!canSave} onClick={() => onSave(password)}>Save password</Button>
                    <Button variant="outline" size="sm" onClick={onCancel}>Cancel (Esc)</Button>
                </span>
            </div>
            <p className="workspace-note">
                {mismatch
                    ? "Passwords don't match. Type the new password again."
                    : mode === "lock" ? "Only this file uses it. You'll need it to open the locked copy." : "Only this file uses it. The batch password is used for the rest."}
            </p>
        </div>
    );
};

export const FilePasswordCell = ({ mode, path, passwords, selected, disabled }: {
    mode: PasswordMode;
    path: string;
    passwords: PasswordState;
    selected: boolean;
    disabled: boolean;
}) => {
    const name = fileName(path);
    const own = path in passwords.overrides;
    const editing = passwords.editing === path;
    return (
        <>
            <span className="file-password">
                {own
                    ? <Badge variant="outline"><KeyRound aria-hidden="true" />Own password</Badge>
                    : <span className="file-password-hint">Batch password</span>}
                {selected && !editing && (own ? (
                    <>
                        <Button variant="ghost" size="xs" aria-label={`Change own password for ${name}`} disabled={disabled} onClick={() => passwords.setEditing(path)}>Change</Button>
                        <Button variant="ghost" size="xs" aria-label={`Use batch password for ${name}`} disabled={disabled} onClick={() => passwords.restoreBatch(path)}>Use batch password</Button>
                    </>
                ) : (
                    <Button variant="ghost" size="xs" aria-label={`Set own password for ${name}`} disabled={disabled} onClick={() => passwords.setEditing(path)}>Set own password</Button>
                ))}
            </span>
            {editing && (
                <OwnPasswordEditor
                    mode={mode}
                    name={name}
                    initial={passwords.overrides[path] ?? ""}
                    onSave={(password) => passwords.saveOverride(path, password)}
                    onCancel={() => passwords.setEditing(null)}
                />
            )}
        </>
    );
};

// The new password is used for this one retry only and is not kept as the file's override.
export const PasswordRetryForm = ({ name, busy, onRetry, onCancel }: {
    name: string;
    busy: boolean;
    onRetry: (password: string) => void;
    onCancel: () => void;
}) => {
    const [password, setPassword] = useState("");
    const id = useId();
    useEffect(() => () => setPassword(""), []);
    return (
        <div
            className="password-inline result-password-retry"
            role="group"
            aria-label={`Retry ${name} with another password`}
            onKeyDown={(event) => {
                if (event.key === "Escape") onCancel();
                if (event.key === "Enter" && password.length > 0 && !busy) onRetry(password);
            }}
        >
            <div className="password-inline-fields">
                <RevealableInput id={`${id}-retry`} label={`Password for ${name}`} value={password} onChange={setPassword} visible={false} placeholder="Enter this file's password" autoFocus />
                <span className="password-inline-actions">
                    <Button size="sm" disabled={busy || password.length === 0} onClick={() => onRetry(password)}>{busy ? "Retrying..." : "Retry this file"}</Button>
                    <Button variant="outline" size="sm" disabled={busy} onClick={onCancel}>Cancel</Button>
                </span>
            </div>
            <p className="workspace-note">Only this file runs again. Its new password isn't saved.</p>
        </div>
    );
};
