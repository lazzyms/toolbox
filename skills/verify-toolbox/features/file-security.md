# File security

File security lets you remove an existing password or protect a PDF or supported Office file with a new password.

## Sub-features

- `password-remove` removes a known password from a PDF or supported Office file.
- `pdf-protect` protects a PDF with a new password.
- `office-protect` protects DOCX or XLSX files with a new password.

## How to get to it (user POV)

- Choose `Documents` under Tool categories and select `Remove Password` or `Protect Office Files`.
- Choose `PDF` under Tool categories and select `Protect PDF`.
- Search `Search tools` for the exact action title.
- Choose a visible action in Quick access or Recent.
- Choose `Open Protect & unlock files` on the Tool library card.

## Driving it with CUA desktop control

Preconditions:

- The native Toolbox window passes Doctor.
- Use only a disposable password-protected fixture copied into the run's `scratch/` directory.
- Record the test password in the local run notes only if the fixture needs one. Never use a personal password.

- **Remove a password.** Open `Remove Password`, choose `Choose files to process`, select the protected scratch file, and enter its known test password. Choose `Remove Password from selected files`. The result list reports completion and names a new unlocked copy. Confirm the output opens with the test password no longer required and the input remains unchanged.
- **Protect a PDF.** Open `Protect PDF`, select a scratch PDF, enter a disposable new password, and choose `Protect`. The result list names a protected copy. Reopen that copy and confirm that the new password works.
- **Protect an Office file.** Open `Protect Office Files`, select a DOCX or XLSX scratch copy, enter a disposable new password, and choose `Protect Office`. Confirm the output exists and opens with the test password. Report unsupported legacy Office formats as failures, not successes.
- **Restore the run.** Keep only the evidence under `evidence/`. Remove scratch inputs and outputs after recording their names and checks.

## Gotchas

- `Remove Password` needs the correct existing password. A wrong password must fail without changing the source.
- `Protect Office Files` supports DOCX and XLSX. Legacy DOC, XLS, and PPT formats are not accepted by that writer.
- The visible result message does not prove that the output is valid. Reopen the output with the expected password behavior.
- Passwords belong only in disposable local fixtures and local evidence notes. Do not put them in the shared skill, Git, or a user-facing report.
