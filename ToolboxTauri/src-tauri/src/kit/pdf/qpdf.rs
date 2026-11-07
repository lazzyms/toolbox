//! The one way to hand the Bundled helper qpdf a password.
//!
//! Passwords never go on qpdf's command line, where other processes can read them, nor into
//! its environment. The caller puts [`PASSWORDS_FROM_STDIN`] (`@-`) where the password options
//! belong, and qpdf reads those options, one per line, from its standard input. Nothing is
//! written to disk, so there is no password file to remove on success, failure or timeout.
//! qpdf echoes some arguments back (an unrecognized option, `--show-encryption`), so every
//! password is masked in what the call returns, its error text included.

use std::process::{Command, Output};
use std::time::Instant;

/// The argument that makes qpdf read the password options from standard input.
pub(crate) const PASSWORDS_FROM_STDIN: &str = "@-";

/// Shown in place of a password in anything qpdf printed.
pub(crate) const MASKED_PASSWORD: &str = "[password hidden]";

pub(crate) const PASSWORD_LINE_BREAK: &str = "Passwords can't contain line breaks.";

/// The qpdf options whose value is a password: the one that opens the input, and the user
/// and owner passwords of `--encrypt`.
const PASSWORD_OPTIONS: [&str; 3] = ["--password", "--user-password", "--owner-password"];

/// Runs `command`, which names [`PASSWORDS_FROM_STDIN`] exactly once, with `passwords` as
/// `(option, password)` pairs. It stops qpdf at `deadline` (or when the job is stopped) and
/// then returns `timeout_message`. No password appears in the returned output or error.
pub(crate) fn run_with_passwords(command: Command, passwords: &[(&str, &str)], deadline: Instant, timeout_message: &str) -> Result<Output, String> {
    let secrets = passwords.iter().map(|(_, password)| *password).collect::<Vec<_>>();
    let input = password_input(&command, passwords)?;
    match super::run_bounded_helper_with_input(command, Some(input), deadline, super::PDF_TEXT_MAX_ERROR_BYTES, super::PDF_TEXT_MAX_ERROR_BYTES, timeout_message) {
        Ok(output) => Ok(Output { status: output.status, stdout: mask(&output.stdout, &secrets), stderr: mask(&output.stderr, &secrets) }),
        Err(message) => Err(String::from_utf8_lossy(&mask(message.as_bytes(), &secrets)).into_owned()),
    }
}

/// The argument file qpdf reads from standard input. Each line is one argument, kept verbatim.
fn password_input(command: &Command, passwords: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    if command.get_args().filter(|argument| *argument == PASSWORDS_FROM_STDIN).count() != 1 {
        return Err(format!("qpdf needs exactly one {PASSWORDS_FROM_STDIN} argument to read passwords from."));
    }
    if passwords.is_empty() {
        return Err("qpdf was asked to read passwords but none were given.".to_string());
    }
    let mut input = String::new();
    for (option, password) in passwords {
        if !PASSWORD_OPTIONS.contains(option) {
            return Err(format!("{option} is not a qpdf password option."));
        }
        if password.contains(['\n', '\r', '\0']) {
            return Err(PASSWORD_LINE_BREAK.to_string());
        }
        input.push_str(option);
        input.push('=');
        input.push_str(password);
        input.push('\n');
    }
    Ok(input.into_bytes())
}

/// Replaces every non-empty secret in `text`, the longest first so one that contains
/// another is hidden whole.
fn mask(text: &[u8], secrets: &[&str]) -> Vec<u8> {
    let mut secrets = secrets.iter().filter(|secret| !secret.is_empty()).map(|secret| secret.as_bytes()).collect::<Vec<_>>();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    let mut masked = text.to_vec();
    for secret in secrets {
        let mut next = Vec::with_capacity(masked.len());
        let mut rest = masked.as_slice();
        while let Some(index) = rest.windows(secret.len()).position(|window| window == secret) {
            next.extend_from_slice(&rest[..index]);
            next.extend_from_slice(MASKED_PASSWORD.as_bytes());
            rest = &rest[index + secret.len()..];
        }
        next.extend_from_slice(rest);
        masked = next;
    }
    masked
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    #[test]
    fn masking_hides_every_password_and_leaves_other_text() {
        let masked = |text: &str, secrets: &[&str]| String::from_utf8(mask(text.as_bytes(), secrets)).unwrap();
        assert_eq!(masked("qpdf: unrecognized argument --owner-password=s3cret", &["s3cret"]), format!("qpdf: unrecognized argument --owner-password={MASKED_PASSWORD}"));
        assert_eq!(masked("ab abc abc", &["ab", "abc"]), format!("{0} {0} {0}", MASKED_PASSWORD));
        assert_eq!(masked("no secret here", &["", "absent"]), "no secret here");
    }

    #[test]
    fn passwords_go_to_standard_input_one_option_per_line() {
        let mut command = Command::new("qpdf");
        command.args(["in.pdf", "out.pdf", "--encrypt", PASSWORDS_FROM_STDIN, "--bits=256", "--"]);
        let input = password_input(&command, &[("--user-password", " sp @ce "), ("--owner-password", "owner")]).unwrap();
        assert_eq!(String::from_utf8(input).unwrap(), "--user-password= sp @ce \n--owner-password=owner\n", "passwords are kept verbatim");
    }

    #[test]
    fn a_password_that_cannot_travel_on_one_line_or_a_misplaced_option_is_refused_without_echoing_it() {
        let with_stdin = || { let mut command = Command::new("qpdf"); command.args([PASSWORDS_FROM_STDIN, "in.pdf"]); command };
        for secret in ["line\nbreak", "carriage\rreturn", "nul\0byte"] {
            let error = password_input(&with_stdin(), &[("--password", secret)]).unwrap_err();
            assert_eq!(error, PASSWORD_LINE_BREAK);
        }
        assert!(!password_input(&with_stdin(), &[("--encrypt", "hunter2")]).unwrap_err().contains("hunter2"));
        assert!(password_input(&with_stdin(), &[]).is_err(), "a call that takes passwords gets at least one");
        let mut on_command_line = Command::new("qpdf");
        on_command_line.args(["--password=hunter2", "in.pdf"]);
        assert!(password_input(&on_command_line, &[("--password", "hunter2")]).is_err(), "without @- qpdf would never read the password");
        let mut twice = Command::new("qpdf");
        twice.args([PASSWORDS_FROM_STDIN, "in.pdf", PASSWORDS_FROM_STDIN]);
        assert!(password_input(&twice, &[("--password", "hunter2")]).is_err());
    }

    // The fake helper is a /bin/sh script, so these checks run on Unix-like targets only.
    #[cfg(unix)]
    mod helper {
        use super::*;
        use crate::kit::common::fake_helper::{secret_exposure, Exposure, FakeHelper};
        use crate::kit::{TEST_OVERRIDE_PASSWORD, TEST_PASSWORD};

        const LONG: Duration = Duration::from_secs(30);

        fn sandbox(name: &str) -> PathBuf {
            let root = std::env::temp_dir().join(format!("toolbox_qpdf_passwords_{}_{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("tmp")).unwrap();
            root
        }

        fn real_qpdf() -> Option<PathBuf> {
            match crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF) {
                Ok(qpdf) => Some(qpdf),
                Err(_) if std::env::var_os("TOOLBOX_REQUIRE_QPDF").is_some() => panic!("qpdf is required for this test run"),
                Err(_) => {
                    eprintln!("skipping: qpdf not available");
                    None
                }
            }
        }

        /// Runs `body` with qpdf resolved through the Development override pointing at `helper`.
        fn with_override<T>(helper: &FakeHelper, body: impl FnOnce(PathBuf) -> T) -> T {
            let previous = std::env::var_os("TOOLBOX_QPDF_PATH");
            std::env::set_var("TOOLBOX_QPDF_PATH", &helper.script);
            let resolved = crate::kit::resources::resolve_helper(&crate::kit::resources::QPDF);
            let result = resolved.map(body);
            match previous {
                Some(value) => std::env::set_var("TOOLBOX_QPDF_PATH", value),
                None => std::env::remove_var("TOOLBOX_QPDF_PATH"),
            }
            result.expect("the Development override resolves to the fake helper")
        }

        /// A qpdf command whose temporary files land inside the sandbox, so the sandbox scan sees them.
        fn qpdf_command(qpdf: &Path, root: &Path) -> Command {
            let mut command = Command::new(qpdf);
            for variable in ["TMPDIR", "TMP", "TEMP"] {
                command.env(variable, root.join("tmp"));
            }
            command
        }

        /// Every file under `root` that holds `secret`: a password file left behind, or a password
        /// written in the clear.
        fn files_holding(root: &Path, secret: &str) -> Vec<PathBuf> {
            let mut found = Vec::new();
            let mut pending = vec![root.to_path_buf()];
            while let Some(directory) = pending.pop() {
                for entry in std::fs::read_dir(&directory).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        pending.push(path);
                    } else if std::fs::read(&path).unwrap().windows(secret.len()).any(|window| window == secret.as_bytes()) {
                        found.push(path);
                    }
                }
            }
            found
        }

        fn opens_with(path: &Path, password: &str) -> bool {
            lopdf::Document::load_with_options(path, lopdf::LoadOptions::with_password(password)).is_ok_and(|document| document.get_pages().len() == 1)
        }

        #[test]
        fn encrypting_rewriting_and_decrypting_keep_every_password_out_of_qpdf_arguments_environment_and_disk() {
            let _guard = crate::kit::lock_process_env();
            let Some(real) = real_qpdf() else { return };
            let root = sandbox("round-trip");
            let plain = root.join("plain.pdf");
            crate::kit::pdf::tests::make_pdf(&plain);
            let (user, owner) = (TEST_PASSWORD, TEST_OVERRIDE_PASSWORD);
            let helper = FakeHelper::install(&root, "qpdf", Some(&real));
            let (encrypted, rewritten, decrypted) = (root.join("encrypted.pdf"), root.join("rewritten.pdf"), root.join("decrypted.pdf"));

            with_override(&helper, |qpdf| {
                let run = |arguments: &[&std::ffi::OsStr], passwords: &[(&str, &str)]| {
                    let mut command = qpdf_command(&qpdf, &root);
                    command.args(arguments);
                    let output = run_with_passwords(command, passwords, Instant::now() + LONG, "timed out").unwrap();
                    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
                };
                run(&[plain.as_os_str(), encrypted.as_os_str(), "--encrypt".as_ref(), PASSWORDS_FROM_STDIN.as_ref(), "--bits=256".as_ref(), "--".as_ref()], &[("--user-password", user), ("--owner-password", owner)]);
                // An encrypted rewrite: qpdf keeps the input's encryption when it opens it with the owner password.
                run(&[PASSWORDS_FROM_STDIN.as_ref(), encrypted.as_os_str(), rewritten.as_os_str()], &[("--password", owner)]);
                run(&[PASSWORDS_FROM_STDIN.as_ref(), "--decrypt".as_ref(), rewritten.as_os_str(), decrypted.as_os_str()], &[("--password", owner)]);
            });

            assert!(opens_with(&encrypted, user) && !opens_with(&encrypted, "wrong-password"), "the encrypted copy opens with its user password only");
            assert!(opens_with(&rewritten, user) && !opens_with(&rewritten, "wrong-password"), "the rewrite keeps the encryption");
            assert!(lopdf::Document::load(&decrypted).is_ok_and(|document| !document.is_encrypted()), "the decrypted copy opens without a password");
            // Tests that do not take the env lock may reach the fake while it is installed, so keep only this run's calls.
            let calls = helper.invocations().into_iter().filter(|call| call.arguments.iter().any(|argument| argument.starts_with(&root.display().to_string()))).collect::<Vec<_>>();
            assert_eq!(calls.len(), 3, "{:?}", calls.iter().map(|call| &call.arguments).collect::<Vec<_>>());
            assert!(calls.iter().all(|call| call.arguments.iter().filter(|argument| *argument == PASSWORDS_FROM_STDIN).count() == 1));
            for secret in [user, owner] {
                assert_eq!(secret_exposure(&calls, secret), Exposure::default(), "a password reached qpdf's arguments or environment");
                assert_eq!(files_holding(&root, secret), Vec::<PathBuf>::new(), "no password is left on disk");
            }
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn a_failed_qpdf_call_never_carries_the_password_in_its_error_text() {
            let _guard = crate::kit::lock_process_env();
            let root = sandbox("echoed");
            // qpdf echoes an unrecognized argument back, password and all; this helper echoes every one.
            // Other calls go to the real qpdf, so tests that do not take the env lock keep working.
            let real = real_qpdf();
            let helper = FakeHelper::install_echoing_input_on(&root, "qpdf", real.as_deref(), "echoes.pdf");

            with_override(&helper, |qpdf| {
                let mut command = qpdf_command(&qpdf, &root);
                command.args([PASSWORDS_FROM_STDIN, "echoes.pdf", "out.pdf"]);
                let output = run_with_passwords(command, &[("--password", TEST_PASSWORD), ("--owner-password", TEST_OVERRIDE_PASSWORD)], Instant::now() + LONG, "timed out").unwrap();
                assert!(!output.status.success());
                for stream in [&output.stdout, &output.stderr] {
                    let text = String::from_utf8_lossy(stream);
                    assert!(text.contains(&format!("--password={MASKED_PASSWORD}")), "the helper did echo its input: {text}");
                    assert!(!text.contains(TEST_PASSWORD) && !text.contains(TEST_OVERRIDE_PASSWORD), "{text}");
                }
            });
            assert_eq!(secret_exposure(&helper.invocations(), TEST_PASSWORD), Exposure::default());
            assert_eq!(files_holding(&root, TEST_PASSWORD), Vec::<PathBuf>::new());
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn protect_reports_a_failed_qpdf_call_without_the_password() {
            let _guard = crate::kit::lock_process_env();
            let root = sandbox("protect-echoed");
            let input = root.join("echoes.pdf");
            crate::kit::pdf::tests::make_pdf(&input);
            let output_folder = root.join("out");
            std::fs::create_dir_all(&output_folder).unwrap();
            let real = real_qpdf();
            let helper = FakeHelper::install_echoing_input_on(&root, "qpdf", real.as_deref(), "echoes.pdf");

            let outcome = with_override(&helper, |_| {
                crate::kit::pdf::PDFProcessor::protect(input.clone(), TEST_PASSWORD, &crate::kit::common::OutputLocation::CustomFolder(output_folder.clone()))
            });
            let error = outcome.error().expect("the failing helper fails the file");
            assert!(error.message.contains(MASKED_PASSWORD), "Protect shows qpdf's own last line: {}", error.message);
            assert!(!error.message.contains(TEST_PASSWORD), "{}", error.message);
            assert!(outcome.output_paths().is_empty());
            assert_eq!(std::fs::read_dir(&output_folder).unwrap().count(), 0, "nothing is published");
            assert_eq!(secret_exposure(&helper.invocations(), TEST_PASSWORD), Exposure::default());
            assert_eq!(files_holding(&root, TEST_PASSWORD), Vec::<PathBuf>::new());
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn a_qpdf_call_stopped_at_its_deadline_leaves_no_password_behind() {
            let _guard = crate::kit::lock_process_env();
            let Some(real) = real_qpdf() else { return };
            let root = sandbox("deadline");
            let input = root.join("stalls.pdf");
            crate::kit::pdf::tests::make_pdf(&input);
            let helper = FakeHelper::install_stalling_on(&root, "qpdf", Some(&real), "stalls.pdf");

            let (result, elapsed) = with_override(&helper, |qpdf| {
                let mut command = qpdf_command(&qpdf, &root);
                command.arg(&input).arg(root.join("partial.pdf")).args(["--encrypt", PASSWORDS_FROM_STDIN, "--bits=256", "--"]);
                let started = Instant::now();
                (run_with_passwords(command, &[("--user-password", TEST_PASSWORD), ("--owner-password", TEST_PASSWORD)], Instant::now() + Duration::from_secs(5), "Locking timed out."), started.elapsed())
            });
            assert_eq!(result.unwrap_err(), "Locking timed out.");
            // The deadline leaves room for a freshly written script to start on a busy macOS host.
            assert!(elapsed < Duration::from_secs(20), "the helper is stopped at its deadline, not waited out");
            assert_eq!(helper.stalled_pids().len(), 1, "the call did reach the stalling helper");
            assert_eq!(secret_exposure(&helper.invocations(), TEST_PASSWORD), Exposure::default());
            assert_eq!(files_holding(&root, TEST_PASSWORD), Vec::<PathBuf>::new());
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn a_refused_password_never_starts_qpdf() {
            let _guard = crate::kit::lock_process_env();
            let root = sandbox("refused");
            let helper = FakeHelper::install(&root, "qpdf", None);
            with_override(&helper, |qpdf| {
                let mut command = qpdf_command(&qpdf, &root);
                command.args([PASSWORDS_FROM_STDIN, "in.pdf"]);
                assert_eq!(run_with_passwords(command, &[("--password", "two\nlines")], Instant::now() + LONG, "timed out").unwrap_err(), PASSWORD_LINE_BREAK);
            });
            assert!(helper.invocations().is_empty());
            let _ = std::fs::remove_dir_all(root);
        }
    }
}
