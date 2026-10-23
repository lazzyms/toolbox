# File security

The File Security view holds Lock & Unlock. Unlock writes a copy that opens without a password. Lock writes a password-protected copy.

## Sub-features

- `lock-unlock` in Unlock mode removes the open password or the permission restrictions from PDF and Office files. Each output ends in `-unlocked`.
- `lock-unlock` in Lock mode protects PDF, DOCX, XLSX and PPTX files with a new password. Each output ends in `-protected`. Legacy DOC, XLS and PPT are accepted and fail per file.

## How to get to it (user POV)

- Choose `File Security` under VIEWS in the sidebar, or `Open File Security` in Tool library.
- Search `Search tools` for `Lock` and choose `Open Lock & Unlock`.
- In the workspace, the `Mode` radio group switches between `Lock` and `Unlock`. Unlock is the default.

## Driving it with CUA desktop control

Preconditions:

- `helpers/doctor.sh` prints `doctor: OK`.
- `helpers/make-fixtures.sh <run-dir>` has written `plain.pdf`, `restricted.pdf` and `legacy.doc` into scratch. `restricted.pdf` is permissions-only with a random owner password nobody keeps.
- The Open panel default points at scratch (see the main skill's Drive section).

- **Unlock a restricted PDF.** Choose `Choose files to process` and pick `restricted.pdf`. The `Files` list shows the row with "It can be unlocked". Type a throwaway value into `Current password for all files`, then choose `Unlock 1 file`. Wait until the progress text "Unlocking 0 of 1" is gone. The result reads "Done. 1 unlocked" and the row reads "Unlocked. Restrictions removed." with `Open restricted-unlocked.pdf` and `Show in Finder: restricted-unlocked.pdf`.
- **Check Unlock on disk.** `qpdf --show-encryption scratch/restricted-unlocked.pdf` prints "File is not encrypted". `qpdf --check` passes. `(cd scratch && shasum -a 256 -c inputs.sha256)` reports every input OK.
- **Lock a PDF.** Switch `Mode` to `Lock` and pick `plain.pdf`. The row reads "It can be locked". Enter one disposable value in `New password for all files` and `Confirm new password`; `Lock 1 file` stays disabled with "1 file has no password yet" until both match. Choose `Lock 1 file`. The result reads "Done. 1 locked" and the row reads "PDF Protected with verified AES-256 encryption" with `Open plain-protected.pdf`.
- **Check Lock on disk.** `qpdf --password=<that value> --show-encryption` shows R = 6, `qpdf --password=<that value> --check` passes, and `qpdf --check` without the password fails with "invalid password". Pass the value on the command line only; never write it down.
- **Lock a legacy file.** Add `legacy.doc` to a Lock batch. That row fails on its own while the other rows still succeed. The Open panel cannot multi-select yet (see the main skill), so this needs one open per file. Last driven before 2026-10-23; source-verified on 2026-10-23.

## Gotchas

- `Unlock 1 file` stays disabled with "1 file has no password yet" even for a permissions-only PDF. The product decision says Unlock needs no password there. Until that lands, type a throwaway value. Report the disabled button as a finding, not a pass.
- After a file loads, "An existing name gets -1, -2 and so on. Nothing is replaced." renders twice in both modes. Observed 2026-10-20 and 2026-10-23.
- The `Open Lock & Unlock` help text still reads "Remove a password you know…" and does not mention Lock.
- The password field is masked. `type_text` may report "0 of N delivered" while the dots appear. Count the dots in a re-read.
- A result message does not prove the output. Always run the qpdf checks.
- Never put a password value in run.md, Git, or a report. Use disposable values only.
