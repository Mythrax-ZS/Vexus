//! Command-line switches for the desktop app (kept dependency-free on purpose).
//!
//! * `--sandbox <dir>`  run the real engine against a fake profile in `<dir>` (created and filled
//!   with sample junk if empty) — safe for demos, screenshots and trying things out
//! * `--portable`       keep all data next to the executable
//! * `--verbose`        log every path touched
//! * `--wait-pid <n>`   internal: wait for another instance to exit (used when restarting elevated)

use std::path::PathBuf;

#[derive(Debug, Default, Clone)]
pub struct Args {
    pub sandbox: Option<PathBuf>,
    pub portable: bool,
    pub verbose: bool,
    pub wait_pid: Option<u32>,
    /// Problems with the command line. A request for a sandbox must never quietly degrade into
    /// running against the real machine, so any error here stops start-up.
    pub errors: Vec<String>,
}

impl Args {
    pub fn parse() -> Args {
        Self::parse_from(std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()))
    }

    pub fn parse_from(args: impl IntoIterator<Item = String>) -> Args {
        let mut out = Args::default();
        let mut it = args.into_iter();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--sandbox" => {
                    match it.next().filter(|v| !v.trim().is_empty() && !v.starts_with("--")) {
                        Some(dir) => out.sandbox = Some(PathBuf::from(dir)),
                        None => out.errors.push(
                            "--sandbox needs a folder, for example: --sandbox C:\\vexus-sandbox"
                                .into(),
                        ),
                    }
                }
                "--portable" => out.portable = true,
                "--verbose" | "-v" => out.verbose = true,
                "--wait-pid" => out.wait_pid = it.next().and_then(|v| v.parse().ok()),
                _ => {}
            }
        }
        out
    }

    /// The switches to pass on when restarting (everything except the internal ones).
    pub fn relaunch_args(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(dir) = &self.sandbox {
            v.push("--sandbox".to_string());
            v.push(dir.display().to_string());
        }
        if self.portable {
            v.push("--portable".to_string());
        }
        if self.verbose {
            v.push("--verbose".to_string());
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &[&str]) -> Args {
        Args::parse_from(s.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_known_switches_and_ignores_the_rest() {
        let a =
            parse(&["--sandbox", r"C:\demo", "--portable", "-v", "--wait-pid", "42", "--bogus"]);
        assert_eq!(a.sandbox, Some(PathBuf::from(r"C:\demo")));
        assert!(a.portable && a.verbose);
        assert_eq!(a.wait_pid, Some(42));
        let none = parse(&[]);
        assert!(none.sandbox.is_none() && !none.portable && none.wait_pid.is_none());
    }

    #[test]
    fn a_bad_wait_pid_is_harmless() {
        let a = parse(&["--wait-pid", "abc"]);
        assert_eq!(a.wait_pid, None);
        assert!(a.errors.is_empty());
    }

    #[test]
    fn a_sandbox_request_without_a_folder_is_an_error_not_a_silent_fallback() {
        for bad in [
            &["--sandbox"][..],
            &["--sandbox", ""],
            &["--sandbox", "--verbose"],
            &["--sandbox", "  "],
        ] {
            let a = parse(bad);
            assert!(a.sandbox.is_none());
            assert_eq!(a.errors.len(), 1, "{bad:?}");
        }
        assert!(parse(&["--sandbox", r"C:\demo"]).errors.is_empty());
    }

    #[test]
    fn relaunch_keeps_user_switches_but_not_internal_ones() {
        let a = parse(&["--sandbox", r"C:\demo", "--wait-pid", "9", "--verbose"]);
        assert_eq!(a.relaunch_args(), ["--sandbox", r"C:\demo", "--verbose"]);
    }
}
