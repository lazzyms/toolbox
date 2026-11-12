# File Security

The File Security workspace has one tool, `Lock & Unlock`. It adds a password to copies of files or removes a known password from copies.

## Sub-features

- `lock` writes a `-protected` copy that opens only with the new password.
- `unlock` writes an `-unlocked` copy that opens without a password.
- `wrong-password` reports a failed file and offers a retry.

## How to get to it (user POV)

- Choose `File Security` in the home rail, or `Open Lock & Unlock` in the Tool library.
- Choose the `Lock` or `Unlock` radio.

## Driving it with CUA desktop control

Preconditions:

- A PDF fixture copy is in `scratch/`.
- Use a disposable password. Never use a real password.

- **Lock.** Choose `Lock`, pick the PDF, and fill `New password for all files` and `Confirm new password`. Choose `Lock 1 file`. Expect `Done. 1 locked` and `<name>-protected.pdf`. `qpdf --show-encryption` reports the file as encrypted.
- **Unlock.** Choose `Unlock`, `Clear list`, pick the locked PDF, fill `Current password for all files`, and choose `Unlock 1 file`. Expect `Done. 1 unlocked`, the status `Unlocked`, and `<name>-unlocked.pdf`. `qpdf --check` opens it without a password.
- **Wrong password.** Unlock the locked PDF with a wrong password. Expect `Done. 0 unlocked, 1 failed`, `Failed, wrong password`, `Retry 1 failed file`, and `Try another password`. No output is written.
- **Not protected.** Unlock a PDF that has no password. Expect `Done. 0 unlocked, 1 skipped` and no copy.

## Gotchas

- Passwords are cleared when you switch mode or leave the workspace.
- The `Show …` checkboxes beside each password field reveal the typed text. Use them to confirm the field before you run.
