//! End-to-end tests of the `init` shell functions in real bash, zsh and fish.
//!
//! A shell that is not installed is skipped with a note on stderr, unless it
//! is listed in `APS_REQUIRE_SHELLS` (comma list), in which case the test
//! fails. `APS_BASH`, `APS_ZSH` and `APS_FISH` override the binaries.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use assert_cmd::Command;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_aws-profile-switcher");

const CONFIG: &str = "\
[default]
[profile dev]
[profile prod]
[profile unset]
";

/// Profile names that would execute code if emitted unquoted.
const HOSTILE: &[&str] = &[
    "q;echo PWNED",
    "x$(id)",
    "it's",
    r"a\b",
    "a b",
    "`id`",
    "$HOME",
    "ünï",
];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Bash,
    Zsh,
    Fish,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Bash => "bash",
            Kind::Zsh => "zsh",
            Kind::Fish => "fish",
        }
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// A5: locate the shell, or skip / fail per `APS_REQUIRE_SHELLS`.
fn shell_bin(kind: Kind) -> Option<PathBuf> {
    let name = kind.name();
    let wanted =
        std::env::var(format!("APS_{}", name.to_uppercase())).unwrap_or_else(|_| name.to_string());
    let required = std::env::var("APS_REQUIRE_SHELLS")
        .unwrap_or_default()
        .split(',')
        .any(|s| s.trim() == name);
    match find_in_path(&wanted) {
        Some(p) => Some(p),
        None if required => {
            panic!("{name} required by APS_REQUIRE_SHELLS but {wanted:?} not found")
        }
        None => {
            eprintln!(
                "skipped: {name} not found ({wanted:?}); set APS_REQUIRE_SHELLS to make this fail"
            );
            None
        }
    }
}

struct Sh {
    kind: Kind,
    path: PathBuf,
    home: TempDir,
}

impl Sh {
    fn new(kind: Kind) -> Option<Self> {
        let path = shell_bin(kind)?;
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join(".aws")).unwrap();
        fs::write(home.path().join(".aws/config"), CONFIG).unwrap();
        let creds: String = HOSTILE.iter().map(|n| format!("[{n}]\n")).collect();
        fs::write(home.path().join(".aws/credentials"), creds).unwrap();
        Some(Sh { kind, path, home })
    }

    /// Loads the function (`init`, extra init args) and runs `body`. The
    /// binary is `$1` / `$argv[1]`; `arg` is `$2` / `$argv[2]`.
    fn run_with(&self, init_args: &str, body: &str, arg: &str, env: &[(&str, &str)]) -> Output {
        let k = self.kind;
        let load = match k {
            Kind::Bash => format!(r#"eval "$("$1" init bash {init_args})""#),
            Kind::Zsh => format!(r#"source <("$1" init zsh {init_args})"#),
            Kind::Fish => format!(r#""$argv[1]" init fish {init_args} | source"#),
        };
        let script = format!("{load}\n{body}\n");
        let mut c = Command::new(&self.path);
        match k {
            Kind::Bash => c.args(["--norc", "--noprofile", "-c", &script, "bash"]),
            Kind::Zsh => c.args(["-f", "-c", &script, "zsh"]),
            Kind::Fish => c.args(["--no-config", "-c", &script]),
        };
        c.arg(BIN)
            .arg(arg)
            .env_clear()
            .env("HOME", self.home.path())
            .env("AWS_CONFIG_FILE", self.home.path().join(".aws/config"))
            .env(
                "AWS_SHARED_CREDENTIALS_FILE",
                self.home.path().join(".aws/credentials"),
            )
            .env("PATH", "/usr/bin:/bin")
            .timeout(Duration::from_secs(20));
        for (key, value) in env {
            c.env(key, value);
        }
        c.output().unwrap()
    }

    /// `posix` is used for bash and zsh, `fish` for fish.
    fn run(&self, posix: &str, fish: &str, env: &[(&str, &str)]) -> Output {
        let body = if self.kind == Kind::Fish { fish } else { posix };
        self.run_with("", body, "", env)
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[track_caller]
fn check(o: &Output, code: i32, stdout: &str) {
    assert_eq!(
        o.status.code(),
        Some(code),
        "stdout: {} stderr: {}",
        out(o),
        err(o)
    );
    assert_eq!(out(o), stdout, "stderr: {}", err(o));
}

/// Prints AWS_PROFILE (or `none`) and exits with the function's status.
const POSIX_SHOW: &str = r#"s=$?; printf %s "${AWS_PROFILE-none}"; exit $s"#;
const FISH_SHOW: &str = r#"set s $status; if set -q AWS_PROFILE; printf %s "$AWS_PROFILE"; else; printf none; end; exit $s"#;

fn function_switches(sh: &Sh) {
    let o = sh.run(
        &format!("awsprofile dev; {POSIX_SHOW}"),
        &format!("awsprofile dev; {FISH_SHOW}"),
        &[],
    );
    check(&o, 0, "dev");
}

fn function_unset_returns_0_when_not_set(sh: &Sh) {
    let o = sh.run(
        &format!("awsprofile unset; {POSIX_SHOW}"),
        &format!("awsprofile unset; {FISH_SHOW}"),
        &[],
    );
    check(&o, 0, "none");
    let o = sh.run(
        &format!("awsprofile unset; {POSIX_SHOW}"),
        &format!("awsprofile unset; {FISH_SHOW}"),
        &[("AWS_PROFILE", "dev")],
    );
    check(&o, 0, "none");
}

fn function_failure_keeps_env(sh: &Sh) {
    let o = sh.run(
        &format!("awsprofile nope; {POSIX_SHOW}"),
        &format!("awsprofile nope; {FISH_SHOW}"),
        &[("AWS_PROFILE", "keep")],
    );
    check(&o, 2, "keep");
}

fn function_passthrough_ls(sh: &Sh) {
    let o = sh.run("awsprofile ls", "awsprofile ls", &[]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(
        out(&o).starts_with("default\ndev\nprod\nunset\n"),
        "{}",
        out(&o)
    );
}

fn function_emit_help_returns_0_without_eval(sh: &Sh) {
    let o = sh.run(
        &format!("awsprofile dev --help; {POSIX_SHOW}"),
        &format!("awsprofile dev --help; {FISH_SHOW}"),
        &[("AWS_PROFILE", "keep")],
    );
    check(&o, 0, "keep");
    assert!(err(&o).contains("Usage"), "{}", err(&o));
}

fn function_clear_env_unset(sh: &Sh) {
    let o = sh.run(
        r#"awsprofile --clear-env unset; s=$?; printf '%s|%s' "${AWS_PROFILE-none}" "${AWS_ACCESS_KEY_ID-none}"; exit $s"#,
        "awsprofile --clear-env unset; set s $status; \
         printf '%s|%s' (set -q AWS_PROFILE; and echo set; or echo none) \
         (set -q AWS_ACCESS_KEY_ID; and echo set; or echo none); exit $s",
        &[("AWS_PROFILE", "dev"), ("AWS_ACCESS_KEY_ID", "AKIAEXAMPLE")],
    );
    check(&o, 0, "none|none");
}

fn function_double_dash(sh: &Sh) {
    let o = sh.run(
        &format!("awsprofile -- unset; {POSIX_SHOW}"),
        &format!("awsprofile -- unset; {FISH_SHOW}"),
        &[],
    );
    check(&o, 0, "unset");
}

fn function_no_args_non_tty_returns_2(sh: &Sh) {
    let o = sh.run(
        &format!("awsprofile; {POSIX_SHOW}"),
        &format!("awsprofile; {FISH_SHOW}"),
        &[],
    );
    check(&o, 2, "none");
    assert!(o.stderr.len() < 4096, "stderr is {} bytes", o.stderr.len());
}

fn hostile_round_trip(sh: &Sh) {
    let posix = format!(r#"awsprofile -- "$2"; {POSIX_SHOW}"#);
    let fish = format!(r#"awsprofile -- $argv[2]; {FISH_SHOW}"#);
    let body = if sh.kind == Kind::Fish { &fish } else { &posix };
    for name in HOSTILE {
        let o = sh.run_with("", body, name, &[]);
        // Exact equality also proves nothing else (e.g. "PWNED") was printed.
        check(&o, 0, name);
        assert!(!err(&o).contains("uid="), "{name}: {}", err(&o));
    }
}

fn custom_name(sh: &Sh) {
    let posix = format!("j dev; {POSIX_SHOW}");
    let fish = format!("j dev; {FISH_SHOW}");
    let body = if sh.kind == Kind::Fish { &fish } else { &posix };
    let o = sh.run_with("--name j", body, "", &[]);
    check(&o, 0, "dev");
}

macro_rules! per_shell {
    ($check:ident => $bash:ident, $zsh:ident, $fish:ident) => {
        #[test]
        fn $bash() {
            if let Some(sh) = Sh::new(Kind::Bash) {
                $check(&sh);
            }
        }
        #[test]
        fn $zsh() {
            if let Some(sh) = Sh::new(Kind::Zsh) {
                $check(&sh);
            }
        }
        #[test]
        fn $fish() {
            if let Some(sh) = Sh::new(Kind::Fish) {
                $check(&sh);
            }
        }
    };
}

per_shell!(function_switches => bash_function_switches, zsh_function_switches, fish_function_switches);
per_shell!(function_unset_returns_0_when_not_set =>
    bash_function_unset_returns_0_when_not_set,
    zsh_function_unset_returns_0_when_not_set,
    fish_function_unset_returns_0_when_not_set);
per_shell!(function_failure_keeps_env =>
    bash_function_failure_keeps_env, zsh_function_failure_keeps_env, fish_function_failure_keeps_env);
per_shell!(function_passthrough_ls =>
    bash_function_passthrough_ls, zsh_function_passthrough_ls, fish_function_passthrough_ls);
per_shell!(function_emit_help_returns_0_without_eval =>
    bash_function_emit_help_returns_0_without_eval,
    zsh_function_emit_help_returns_0_without_eval,
    fish_function_emit_help_returns_0_without_eval);
per_shell!(function_clear_env_unset =>
    bash_function_clear_env_unset, zsh_function_clear_env_unset, fish_function_clear_env_unset);
per_shell!(function_double_dash =>
    bash_function_double_dash, zsh_function_double_dash, fish_function_double_dash);
per_shell!(function_no_args_non_tty_returns_2 =>
    bash_function_no_args_non_tty_returns_2,
    zsh_function_no_args_non_tty_returns_2,
    fish_function_no_args_non_tty_returns_2);
per_shell!(hostile_round_trip => bash_hostile_round_trip, zsh_hostile_round_trip, fish_hostile_round_trip);
per_shell!(custom_name => bash_custom_name, zsh_custom_name, fish_custom_name);
