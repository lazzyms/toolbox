//! A fake external helper for tests, installed through a Development override such as
//! `TOOLBOX_QPDF_PATH`. It records each call's arguments and environment, then hands the
//! call to the real helper when one is given, so other tests that find it keep working.
//!
//! The fake is a `/bin/sh` script, so it exists on Unix-like targets only. Windows runs
//! compile none of these checks.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub struct FakeHelper {
    pub script: PathBuf,
    records: PathBuf,
}

#[derive(Debug)]
pub struct Invocation {
    pub arguments: Vec<String>,
    pub environment: String,
}

/// Where a secret reached a helper. Both must stay false for a password.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Exposure {
    pub in_arguments: bool,
    pub in_environment: bool,
}

impl FakeHelper {
    pub fn install(directory: &Path, name: &str, delegate: Option<&Path>) -> Self {
        let records = directory.join(format!("{name}-calls"));
        std::fs::create_dir_all(&records).unwrap();
        let script = directory.join(name);
        let hand_off = match delegate {
            Some(real) => format!("exec '{}' \"$@\"", real.display()),
            None => "exit 0".to_string(),
        };
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nrecord='{}'/call-$$\nprintf '%s\\n' \"$@\" > \"$record.args\"\nenv > \"$record.env\"\n{hand_off}\n",
                records.display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();
        Self { script, records }
    }

    pub fn invocations(&self) -> Vec<Invocation> {
        let mut calls = std::fs::read_dir(&self.records)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "args"))
            .collect::<Vec<_>>();
        calls.sort();
        calls
            .into_iter()
            .map(|arguments| Invocation {
                arguments: std::fs::read_to_string(&arguments).unwrap().lines().map(str::to_string).collect(),
                environment: std::fs::read_to_string(arguments.with_extension("env")).unwrap_or_default(),
            })
            .collect()
    }
}

/// The shared check: reports whether `secret` reached any recorded call's arguments or environment.
pub fn secret_exposure(invocations: &[Invocation], secret: &str) -> Exposure {
    assert!(!secret.is_empty(), "an empty secret matches everything");
    Exposure {
        in_arguments: invocations.iter().any(|call| call.arguments.iter().any(|argument| argument.contains(secret))),
        in_environment: invocations.iter().any(|call| call.environment.contains(secret)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("toolbox_fake_helper_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn the_shared_check_finds_a_secret_in_arguments_or_environment_and_passes_a_clean_call() {
        let root = directory("self-test");
        let helper = FakeHelper::install(&root, "helper", None);

        Command::new(&helper.script).args(["--encrypt", "pa55-in-argv", "--", "in.pdf"]).status().unwrap();
        assert_eq!(
            secret_exposure(&helper.invocations(), "pa55-in-argv"),
            Exposure { in_arguments: true, in_environment: false }
        );

        Command::new(&helper.script).arg("in.pdf").env("TOOLBOX_TEST_SECRET", "pa55-in-env").status().unwrap();
        assert_eq!(
            secret_exposure(&helper.invocations(), "pa55-in-env"),
            Exposure { in_arguments: false, in_environment: true }
        );

        assert_eq!(secret_exposure(&helper.invocations(), "never-passed"), Exposure::default());
        assert_eq!(helper.invocations().len(), 2);

        std::fs::remove_dir_all(root).unwrap();
    }
}
