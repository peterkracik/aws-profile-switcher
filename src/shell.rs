use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::AppError;

/// Output dialect. `Posix`, `Bash` and `Zsh` emit the same code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Shell {
    Posix,
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    /// Parses `AWS_PROFILE_SWITCHER_SHELL`: empty means unset, unknown is an error.
    pub fn parse_env(value: &str) -> Result<Option<Shell>, AppError> {
        if value.is_empty() {
            return Ok(None);
        }
        <Shell as clap::ValueEnum>::from_str(value, false)
            .map(Some)
            .map_err(|_| {
                AppError::Usage(format!(
                    "invalid {}={value:?} (expected posix, bash, zsh or fish)",
                    crate::env::SWITCHER_SHELL_VAR
                ))
            })
    }
}

/// Shell code safe to eval. Only `emit_*` in this module can build one, so
/// every string that reaches stdout for eval went through `quote`.
#[derive(Debug, PartialEq, Eq)]
pub struct ShellScript(String);

impl ShellScript {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Single-quotes `s` so the target shell reads it back byte-for-byte.
pub fn quote(shell: Shell, s: &str) -> String {
    match shell {
        // fish single quotes only interpret \\ and \'.
        Shell::Fish => format!("'{}'", s.replace('\\', r"\\").replace('\'', r"\'")),
        // POSIX single quotes interpret nothing; close, escape, reopen.
        _ => format!("'{}'", s.replace('\'', r"'\''")),
    }
}

const CREDS: &str = "AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN";

/// One line, `;`-separated, ending in exactly one `\n`.
pub fn emit_switch(shell: Shell, profile: &str, clear_env: bool) -> ShellScript {
    let q = quote(shell, profile);
    let (clear, set) = match shell {
        Shell::Fish => ("set -e", format!("set -gx AWS_PROFILE {q}")),
        _ => ("unset", format!("export AWS_PROFILE={q}")),
    };
    ShellScript(if clear_env {
        format!("{clear} {CREDS}; {set}\n")
    } else {
        format!("{set}\n")
    })
}

/// fish appends `; true` because `set -e` returns 4 for a missing variable.
pub fn emit_unset(shell: Shell, clear_env: bool) -> ShellScript {
    let vars = if clear_env {
        format!("AWS_PROFILE {CREDS}")
    } else {
        "AWS_PROFILE".to_string()
    };
    ShellScript(match shell {
        Shell::Fish => format!("set -e {vars}; true\n"),
        _ => format!("unset {vars}\n"),
    })
}

const POSIX_INIT: &str = include_str!("init/posix.sh");
const FISH_INIT: &str = include_str!("init/fish.fish");

/// The shell function for `init`. Only bash, zsh and fish have templates.
pub fn init_script(shell: Shell, name: &str, exe: &Path) -> Result<String, AppError> {
    let (template, shell_name) = match shell {
        Shell::Bash => (POSIX_INIT, "bash"),
        Shell::Zsh => (POSIX_INIT, "zsh"),
        Shell::Fish => (FISH_INIT, "fish"),
        Shell::Posix => {
            return Err(AppError::Usage(
                "init supports bash, zsh and fish".to_string(),
            ));
        }
    };
    if !valid_function_name(name) {
        return Err(AppError::Usage(format!(
            "invalid --name {name:?}: must match ^[A-Za-z_][A-Za-z0-9_-]*$"
        )));
    }
    let exe = exe.to_str().ok_or_else(|| {
        AppError::Usage(format!("binary path is not valid UTF-8: {}", exe.display()))
    })?;
    Ok(template
        .replace("{{NAME}}", name)
        .replace("{{SHELL}}", shell_name)
        .replace("{{EXE}}", &quote(shell, exe)))
}

/// The path `init` embeds. Package managers (Homebrew) run the binary from a
/// versioned dir that vanishes on upgrade, so prefer the first absolute PATH
/// entry with the same file name that resolves to the same file (e.g. the
/// `/opt/homebrew/bin` symlink). Falls back to `exe`.
pub fn stable_exe(exe: &Path, path_var: Option<&OsStr>) -> PathBuf {
    let (Some(name), Some(path_var)) = (exe.file_name(), path_var) else {
        return exe.to_path_buf();
    };
    let Ok(real) = exe.canonicalize() else {
        return exe.to_path_buf();
    };
    std::env::split_paths(path_var)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|cand| cand.canonicalize().is_ok_and(|c| c == real))
        .unwrap_or_else(|| exe.to_path_buf())
}

/// `^[A-Za-z_][A-Za-z0-9_-]*$`, hand-written to avoid a regex dependency.
fn valid_function_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSIXISH: [Shell; 3] = [Shell::Posix, Shell::Bash, Shell::Zsh];

    #[test]
    fn quote_posix() {
        for sh in POSIXISH {
            assert_eq!(quote(sh, "it's"), r"'it'\''s'");
            assert_eq!(quote(sh, r"a\b"), r"'a\b'");
            assert_eq!(quote(sh, "q;echo PWNED"), "'q;echo PWNED'");
            assert_eq!(quote(sh, "x$(id)`id`"), "'x$(id)`id`'");
            assert_eq!(quote(sh, ""), "''");
        }
    }

    #[test]
    fn quote_fish() {
        assert_eq!(quote(Shell::Fish, "it's"), r"'it\'s'");
        assert_eq!(quote(Shell::Fish, r"a\b"), r"'a\\b'");
        assert_eq!(quote(Shell::Fish, "x$(id)"), "'x$(id)'");
    }

    #[test]
    fn emit_switch_posix() {
        for sh in POSIXISH {
            assert_eq!(
                emit_switch(sh, "x", false).as_str(),
                "export AWS_PROFILE='x'\n"
            );
            assert_eq!(
                emit_switch(sh, "x", true).as_str(),
                "unset AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN; export AWS_PROFILE='x'\n"
            );
            assert_eq!(
                emit_switch(sh, "it's", false).as_str(),
                "export AWS_PROFILE='it'\\''s'\n"
            );
        }
    }

    #[test]
    fn emit_switch_fish() {
        assert_eq!(
            emit_switch(Shell::Fish, "x", false).as_str(),
            "set -gx AWS_PROFILE 'x'\n"
        );
        assert_eq!(
            emit_switch(Shell::Fish, "x", true).as_str(),
            "set -e AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN; set -gx AWS_PROFILE 'x'\n"
        );
    }

    #[test]
    fn emit_unset_posix() {
        for sh in POSIXISH {
            assert_eq!(emit_unset(sh, false).as_str(), "unset AWS_PROFILE\n");
            assert_eq!(
                emit_unset(sh, true).as_str(),
                "unset AWS_PROFILE AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN\n"
            );
        }
    }

    #[test]
    fn emit_unset_fish() {
        assert_eq!(
            emit_unset(Shell::Fish, false).as_str(),
            "set -e AWS_PROFILE; true\n"
        );
        assert_eq!(
            emit_unset(Shell::Fish, true).as_str(),
            "set -e AWS_PROFILE AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN; true\n"
        );
    }

    #[test]
    fn shell_parse() {
        assert_eq!(Shell::parse_env("zsh").unwrap(), Some(Shell::Zsh));
        assert_eq!(Shell::parse_env("bash").unwrap(), Some(Shell::Bash));
        assert_eq!(Shell::parse_env("fish").unwrap(), Some(Shell::Fish));
        assert_eq!(Shell::parse_env("posix").unwrap(), Some(Shell::Posix));
        assert_eq!(Shell::parse_env("").unwrap(), None);
        assert!(matches!(Shell::parse_env("tcsh"), Err(AppError::Usage(_))));
    }

    mod init {
        use super::super::*;

        const EXE: &str = "/usr/local/bin/aws-profile-switcher";

        #[test]
        fn init_substitutes_all() {
            for sh in [Shell::Bash, Shell::Zsh, Shell::Fish] {
                let out = init_script(sh, "awsprofile", Path::new(EXE)).unwrap();
                assert!(!out.contains("{{"), "{sh:?}: {out}");
                assert!(out.contains(&format!("command '{EXE}'")), "{sh:?}: {out}");
                assert!(out.ends_with('\n'));
            }
            let fish = init_script(Shell::Fish, "awsprofile", Path::new(EXE)).unwrap();
            assert!(fish.contains("function awsprofile\n"), "{fish}");
            let bash = init_script(Shell::Bash, "awsprofile", Path::new(EXE)).unwrap();
            assert!(bash.contains("awsprofile() {"), "{bash}");
            assert!(
                bash.contains("AWS_PROFILE_SWITCHER_SHELL=bash command"),
                "{bash}"
            );
        }

        #[test]
        fn init_quotes_exe_with_space() {
            let out = init_script(Shell::Zsh, "awsprofile", Path::new("/tmp/a b/aps")).unwrap();
            assert!(out.contains("command '/tmp/a b/aps'"), "{out}");
            let out = init_script(Shell::Fish, "awsprofile", Path::new("/tmp/it's/aps")).unwrap();
            assert!(out.contains(r"command '/tmp/it\'s/aps'"), "{out}");
        }

        #[test]
        fn init_sets_emit_var() {
            let out = init_script(Shell::Zsh, "awsprofile", Path::new(EXE)).unwrap();
            assert!(out.contains("AWS_PROFILE_SWITCHER_SHELL=zsh"), "{out}");
        }

        #[test]
        fn init_name_valid() {
            for name in ["awsprofile", "j", "_x-1"] {
                let out = init_script(Shell::Zsh, name, Path::new(EXE)).unwrap();
                assert!(out.contains(&format!("{name}() {{")), "{out}");
            }
        }

        #[test]
        fn init_name_invalid() {
            for name in ["a;b", "1x", "-x", "", "a b", "ä"] {
                assert!(
                    matches!(
                        init_script(Shell::Zsh, name, Path::new(EXE)),
                        Err(AppError::Usage(_))
                    ),
                    "{name:?}"
                );
            }
        }

        #[test]
        fn init_rejects_posix() {
            assert!(matches!(
                init_script(Shell::Posix, "awsprofile", Path::new(EXE)),
                Err(AppError::Usage(_))
            ));
        }

        mod stable {
            use super::super::super::*;
            use std::ffi::OsString;
            use std::fs;
            use std::os::unix::fs::symlink;
            use std::path::PathBuf;

            const BIN: &str = "aws-profile-switcher";

            /// `cellar/aws-profile-switcher` (the real binary) plus empty
            /// `link/` and `other/` dirs.
            fn layout() -> (tempfile::TempDir, PathBuf) {
                let dir = tempfile::tempdir().unwrap();
                for d in ["cellar", "link", "other", "link2"] {
                    fs::create_dir(dir.path().join(d)).unwrap();
                }
                let exe = dir.path().join("cellar").join(BIN);
                fs::write(&exe, "bin").unwrap();
                (dir, exe)
            }

            fn path_of(dirs: &[PathBuf]) -> OsString {
                std::env::join_paths(dirs).unwrap()
            }

            #[test]
            fn stable_exe_prefers_path_symlink() {
                let (dir, exe) = layout();
                let link = dir.path().join("link");
                let link2 = dir.path().join("link2");
                symlink(&exe, link.join(BIN)).unwrap();
                symlink(&exe, link2.join(BIN)).unwrap();
                let path = path_of(&[dir.path().join("other"), link.clone(), link2]);
                assert_eq!(stable_exe(&exe, Some(&path)), link.join(BIN));
            }

            #[test]
            fn stable_exe_no_match_falls_back() {
                let (dir, exe) = layout();
                let path = path_of(&[dir.path().join("other"), dir.path().join("link")]);
                assert_eq!(stable_exe(&exe, Some(&path)), exe);
            }

            #[test]
            fn stable_exe_ignores_other_same_name_binary() {
                let (dir, exe) = layout();
                let other = dir.path().join("other");
                fs::write(other.join(BIN), "different").unwrap();
                let path = path_of(std::slice::from_ref(&other));
                assert_eq!(stable_exe(&exe, Some(&path)), exe);
            }

            #[test]
            fn stable_exe_empty_or_missing_path() {
                let (_dir, exe) = layout();
                assert_eq!(stable_exe(&exe, Some(&OsString::new())), exe);
                assert_eq!(stable_exe(&exe, None), exe);
            }

            #[test]
            fn stable_exe_direct_path_entry_is_itself() {
                // Binary not under Homebrew: its own dir is on PATH.
                let (dir, exe) = layout();
                let path = path_of(&[dir.path().join("cellar")]);
                assert_eq!(stable_exe(&exe, Some(&path)), exe);
            }
        }

        #[test]
        fn init_rejects_non_utf8_exe() {
            use std::os::unix::ffi::OsStrExt;
            let exe = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff/aps"));
            assert!(matches!(
                init_script(Shell::Zsh, "awsprofile", exe),
                Err(AppError::Usage(_))
            ));
        }
    }
}
