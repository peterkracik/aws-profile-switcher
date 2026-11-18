//! End-to-end tests of the binary. Every test runs with a cleared environment
//! and a temporary HOME; nothing touches the real ~/.aws.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_aws-profile-switcher");

const STD_CONFIG: &str = "\
[default]
region = eu-west-1
[profile dev]
[profile my-dev]
[profile prod]
";
const STD_CREDS: &str = "\
[ci]
aws_access_key_id = AKIAEXAMPLE
aws_secret_access_key = SECRETVALUE
";

const SSO_CONFIG: &str = "\
[profile s]
sso_session = my-sso
sso_account_id = 111111111111
[sso-session my-sso]
sso_start_url = https://example.awsapps.com/start
[profile legacy]
sso_start_url = https://example.awsapps.com/start
[profile dev]
";
const MY_SSO_KEY: &str = "0ad374308c5a4e22f723adf10145eafad7c4031c";
const START_URL_KEY: &str = "e8be5486177c5b5392bd9aa76563515b29358e6e";
const VALID: &str = r#"{"accessToken":"t","expiresAt":"2999-01-01T00:00:00Z"}"#;
const EXPIRED: &str = r#"{"accessToken":"t","expiresAt":"2000-01-01T00:00:00Z"}"#;

const FAKE_AWS: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$HOME/aws-called"
printf 'shell=%s\n' "${AWS_PROFILE_SWITCHER_SHELL-unset}" >> "$HOME/aws-env"
echo CODE-1234
echo boom >&2
exit "${FAKE_AWS_RC:-0}"
"#;

struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new(config: &str, credentials: &str) -> Self {
        Self::with_bytes(config.as_bytes(), credentials)
    }

    fn with_bytes(config: &[u8], credentials: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let f = Fixture { dir };
        fs::create_dir_all(f.home().join(".aws/sso/cache")).unwrap();
        fs::create_dir_all(f.home().join("bin")).unwrap();
        fs::write(f.config(), config).unwrap();
        fs::write(f.credentials(), credentials).unwrap();
        let aws = f.home().join("bin/aws");
        fs::write(&aws, FAKE_AWS).unwrap();
        fs::set_permissions(&aws, fs::Permissions::from_mode(0o755)).unwrap();
        f
    }

    fn std() -> Self {
        Self::new(STD_CONFIG, STD_CREDS)
    }

    fn sso() -> Self {
        Self::new(SSO_CONFIG, "")
    }

    fn home(&self) -> &Path {
        self.dir.path()
    }

    fn config(&self) -> PathBuf {
        self.home().join(".aws/config")
    }

    fn credentials(&self) -> PathBuf {
        self.home().join(".aws/credentials")
    }

    fn cache(&self, key: &str, json: &str) {
        fs::write(self.home().join(format!(".aws/sso/cache/{key}.json")), json).unwrap();
    }

    fn aws_called(&self) -> Option<String> {
        fs::read_to_string(self.home().join("aws-called")).ok()
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(BIN);
        c.env_clear()
            .env("HOME", self.home())
            .env("AWS_CONFIG_FILE", self.config())
            .env("AWS_SHARED_CREDENTIALS_FILE", self.credentials())
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.home().join("bin").display()),
            )
            .timeout(Duration::from_secs(10));
        c
    }
}

fn run(mut c: Command, args: &[&str]) -> Output {
    c.args(args).output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[track_caller]
fn assert_ok(o: &Output, want_stdout: &str) {
    assert_eq!(o.status.code(), Some(0), "stderr: {}", stderr(o));
    assert_eq!(stdout(o), want_stdout, "stderr: {}", stderr(o));
}

#[track_caller]
fn assert_fails(o: &Output, code: i32) {
    assert_eq!(o.status.code(), Some(code), "stderr: {}", stderr(o));
    assert!(o.stdout.is_empty(), "stdout must be empty: {:?}", stdout(o));
}

#[test]
fn fixture_paths_inside_tempdir() {
    let f = Fixture::std();
    let tmp = std::env::temp_dir().canonicalize().unwrap();
    let home = f.home().canonicalize().unwrap();
    assert!(home.starts_with(&tmp), "{home:?} not under {tmp:?}");
    for p in [f.config(), f.credentials()] {
        assert!(p.canonicalize().unwrap().starts_with(&home), "{p:?}");
    }
}

// ---- switching ----

#[test]
fn switch_exact_posix_default() {
    let f = Fixture::std();
    assert_ok(&run(f.cmd(), &["dev"]), "export AWS_PROFILE='dev'\n");
}

#[test]
fn switch_emit_fish() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_PROFILE_SWITCHER_SHELL", "fish");
    assert_ok(&run(c, &["dev"]), "set -gx AWS_PROFILE 'dev'\n");
}

#[test]
fn switch_shell_flag_overrides_env() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_PROFILE_SWITCHER_SHELL", "zsh");
    assert_ok(
        &run(c, &["--shell", "fish", "dev"]),
        "set -gx AWS_PROFILE 'dev'\n",
    );
}

#[test]
fn invalid_emit_shell_env_exits_2() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_PROFILE_SWITCHER_SHELL", "tcsh");
    let o = run(c, &["dev"]);
    assert_fails(&o, 2);
    assert!(
        stderr(&o).contains("AWS_PROFILE_SWITCHER_SHELL"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn unique_substring_autoselects_with_note() {
    let f = Fixture::std();
    let o = run(f.cmd(), &["PRO"]);
    assert_ok(&o, "export AWS_PROFILE='prod'\n");
    assert!(stderr(&o).contains("prod"), "{}", stderr(&o));
}

#[test]
fn ambiguous_without_tty_exits_2() {
    let f = Fixture::std();
    let o = run(f.cmd(), &["d"]);
    assert_fails(&o, 2);
    assert!(stderr(&o).contains("terminal"), "{}", stderr(&o));
}

#[test]
fn unknown_without_tty_exits_2() {
    let f = Fixture::std();
    let o = run(f.cmd(), &["zzz"]);
    assert_fails(&o, 2);
    assert!(stderr(&o).contains("zzz"), "{}", stderr(&o));
}

#[test]
fn no_args_without_tty_fails_fast() {
    let f = Fixture::std();
    let start = Instant::now();
    let o = run(f.cmd(), &[]);
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_fails(&o, 2);
    assert!(o.stderr.len() < 4096, "stderr is {} bytes", o.stderr.len());
    assert!(stderr(&o).contains("terminal"), "{}", stderr(&o));
}

#[test]
fn empty_profiles_exit_2_clear_message() {
    let f = Fixture::new("", "");
    let o = run(f.cmd(), &["ls"]);
    assert_fails(&o, 2);
    let e = stderr(&o);
    assert!(e.contains("no AWS profiles found"), "{e}");
    assert!(e.contains(&f.config().display().to_string()), "{e}");
    assert!(e.contains(&f.credentials().display().to_string()), "{e}");
    assert_fails(&run(f.cmd(), &[]), 2);
}

#[test]
fn invalid_utf8_config_does_not_panic() {
    let f = Fixture::with_bytes(b"[profile \xff\xfe]\n[profile dev]\n", "");
    assert_ok(&run(f.cmd(), &["dev"]), "export AWS_PROFILE='dev'\n");
}

#[test]
fn ls_works_without_aws_on_path() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("PATH", "/usr/bin:/bin");
    assert_ok(&run(c, &["ls"]), "default\ndev\nmy-dev\nprod\nci\n");
}

#[test]
fn static_creds_warn_without_clear_env() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_ACCESS_KEY_ID", "AKIAEXAMPLE");
    let o = run(c, &["dev"]);
    assert_ok(&o, "export AWS_PROFILE='dev'\n");
    let e = stderr(&o);
    assert!(
        e.contains("AWS_ACCESS_KEY_ID") && e.contains("--clear-env"),
        "{e}"
    );
    assert!(!e.contains("AKIAEXAMPLE"), "must not echo values: {e}");
}

#[test]
fn clear_env_emits_unsets() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_ACCESS_KEY_ID", "AKIAEXAMPLE");
    let o = run(c, &["--clear-env", "dev"]);
    assert_ok(
        &o,
        "unset AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN; export AWS_PROFILE='dev'\n",
    );
    assert!(!stderr(&o).contains("warning"), "{}", stderr(&o));
}

// ---- parsing regressions ----

#[test]
fn clear_env_before_unset_runs_unset() {
    let f = Fixture::std();
    assert_ok(
        &run(f.cmd(), &["--clear-env", "unset"]),
        "unset AWS_PROFILE AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN\n",
    );
}

#[test]
fn login_before_ls_runs_ls() {
    let f = Fixture::std();
    assert_ok(
        &run(f.cmd(), &["--login", "ls"]),
        "default\ndev\nmy-dev\nprod\nci\n",
    );
    assert_eq!(f.aws_called(), None);
}

#[test]
fn double_dash_forces_profile_named_unset() {
    let f = Fixture::new("[profile unset]\n[profile dev]\n", "");
    assert_ok(
        &run(f.cmd(), &["--", "unset"]),
        "export AWS_PROFILE='unset'\n",
    );
}

#[test]
fn profile_plus_subcommand_is_usage_error() {
    let f = Fixture::std();
    assert_fails(&run(f.cmd(), &["dev", "ls"]), 2);
}

#[test]
fn emit_mode_help_goes_to_stderr() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_PROFILE_SWITCHER_SHELL", "zsh");
    let o = run(c, &["dev", "--help"]);
    assert_ok(&o, "");
    assert!(stderr(&o).contains("Usage"), "{}", stderr(&o));
    let o = run(f.cmd(), &["--shell", "zsh", "unset", "--help"]);
    assert_ok(&o, "");
    assert!(stderr(&o).contains("Usage"), "{}", stderr(&o));
}

#[test]
fn emit_mode_version_goes_to_stderr() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_PROFILE_SWITCHER_SHELL", "bash");
    let o = run(c, &["--version"]);
    assert_ok(&o, "");
    assert!(
        stderr(&o).contains(env!("CARGO_PKG_VERSION")),
        "{}",
        stderr(&o)
    );
}

#[test]
fn emit_mode_ls_goes_to_stderr() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_PROFILE_SWITCHER_SHELL", "zsh");
    let o = run(c, &["ls"]);
    assert_ok(&o, "");
    assert!(stderr(&o).contains("my-dev"), "{}", stderr(&o));
    let o = run(f.cmd(), &["--shell", "fish", "ls"]);
    assert_ok(&o, "");
    assert!(stderr(&o).contains("my-dev"), "{}", stderr(&o));
}

#[test]
fn non_emit_help_on_stdout() {
    let f = Fixture::std();
    let o = run(f.cmd(), &["--help"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).contains("Usage"), "{}", stdout(&o));
    let o = run(f.cmd(), &["--version"]);
    assert_ok(
        &o,
        &format!("aws-profile-switcher {}\n", env!("CARGO_PKG_VERSION")),
    );
}

// ---- unset / current ----

#[test]
fn unset_posix() {
    let f = Fixture::std();
    assert_ok(&run(f.cmd(), &["unset"]), "unset AWS_PROFILE\n");
}

#[test]
fn unset_fish() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_PROFILE_SWITCHER_SHELL", "fish");
    assert_ok(&run(c, &["unset"]), "set -e AWS_PROFILE; true\n");
}

#[test]
fn current_prints_profile() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_PROFILE", "dev");
    assert_ok(&run(c, &["current"]), "dev\n");
}

#[test]
fn current_unset_exits_1() {
    let f = Fixture::std();
    let o = run(f.cmd(), &["current"]);
    assert_fails(&o, 1);
    assert!(
        stderr(&o).contains("AWS_PROFILE is not set"),
        "{}",
        stderr(&o)
    );
}

// ---- init ----

#[test]
fn init_zsh_contains_abs_exe() {
    let f = Fixture::std();
    let o = run(f.cmd(), &["init", "zsh"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let out = stdout(&o);
    let canon = fs::canonicalize(BIN).unwrap();
    assert!(
        out.contains(&format!("command '{BIN}'"))
            || out.contains(&format!("command '{}'", canon.display())),
        "{out}"
    );
    assert!(
        out.contains("AWS_PROFILE_SWITCHER_SHELL=zsh command"),
        "{out}"
    );
    assert!(out.contains("awsprofile() {"), "{out}");
}

#[test]
fn init_prefers_path_symlink() {
    // Homebrew layout: PATH has a symlink to the versioned Cellar binary.
    let f = Fixture::std();
    let link_dir = f.home().join("brew-bin");
    fs::create_dir(&link_dir).unwrap();
    let link = link_dir.join("aws-profile-switcher");
    std::os::unix::fs::symlink(BIN, &link).unwrap();
    let mut c = f.cmd();
    c.env("PATH", format!("{}:/usr/bin:/bin", link_dir.display()));
    let o = run(c, &["init", "zsh"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(
        out.contains(&format!("command '{}'", link.display())),
        "{out}"
    );
}

#[test]
fn init_bad_name_exits_2() {
    let f = Fixture::std();
    assert_fails(&run(f.cmd(), &["init", "zsh", "--name", "a;b"]), 2);
}

#[test]
fn init_posix_rejected() {
    let f = Fixture::std();
    assert_fails(&run(f.cmd(), &["init", "posix"]), 2);
}

// ---- SSO ----

#[test]
fn sso_valid_token_does_not_run_aws() {
    let f = Fixture::sso();
    f.cache(MY_SSO_KEY, VALID);
    let o = run(f.cmd(), &["s"]);
    assert_ok(&o, "export AWS_PROFILE='s'\n");
    assert_eq!(f.aws_called(), None);
    assert!(!stderr(&o).contains("warning"), "{}", stderr(&o));
}

#[test]
fn sso_expired_no_tty_switches_with_warning() {
    let f = Fixture::sso();
    f.cache(MY_SSO_KEY, EXPIRED);
    let o = run(f.cmd(), &["s"]);
    assert_ok(&o, "export AWS_PROFILE='s'\n");
    assert_eq!(f.aws_called(), None);
    let e = stderr(&o);
    assert!(e.contains("warning") && e.contains("expired"), "{e}");
    // Missing cache file: same, with a different reason.
    let f = Fixture::sso();
    let o = run(f.cmd(), &["s"]);
    assert_ok(&o, "export AWS_PROFILE='s'\n");
    assert!(stderr(&o).contains("no cached SSO token"), "{}", stderr(&o));
}

#[test]
fn sso_expired_with_login_keeps_stdout_clean() {
    let f = Fixture::sso();
    f.cache(MY_SSO_KEY, EXPIRED);
    let mut c = f.cmd();
    c.env("AWS_PROFILE_SWITCHER_SHELL", "zsh");
    let o = run(c, &["--login", "s"]);
    assert_ok(&o, "export AWS_PROFILE='s'\n");
    assert!(stderr(&o).contains("CODE-1234"), "{}", stderr(&o));
    assert_eq!(f.aws_called().as_deref(), Some("sso login --profile s\n"));
    assert_eq!(
        fs::read_to_string(f.home().join("aws-env")).unwrap(),
        "shell=unset\n"
    );
}

#[test]
fn sso_login_failure_exits_2_and_shows_aws_stderr() {
    let f = Fixture::sso();
    let mut c = f.cmd();
    c.env("FAKE_AWS_RC", "1");
    let o = run(c, &["--login", "s"]);
    assert_fails(&o, 2);
    assert!(stderr(&o).contains("boom"), "{}", stderr(&o));
}

#[test]
fn sso_login_without_aws_exits_2() {
    let f = Fixture::sso();
    let mut c = f.cmd();
    c.env("PATH", "/usr/bin:/bin");
    let o = run(c, &["--login", "s"]);
    assert_fails(&o, 2);
    assert!(stderr(&o).contains("aws"), "{}", stderr(&o));
}

#[test]
fn sso_no_sso_never_runs_aws() {
    let f = Fixture::sso();
    let o = run(f.cmd(), &["--no-sso", "s"]);
    assert_ok(&o, "export AWS_PROFILE='s'\n");
    assert_eq!(f.aws_called(), None);
    assert!(!stderr(&o).contains("warning"), "{}", stderr(&o));
}

#[test]
fn legacy_start_url_profile_uses_start_url_hash() {
    let f = Fixture::sso();
    f.cache(START_URL_KEY, VALID);
    let o = run(f.cmd(), &["legacy"]);
    assert_ok(&o, "export AWS_PROFILE='legacy'\n");
    assert!(!stderr(&o).contains("warning"), "{}", stderr(&o));
    // The session-keyed file does not satisfy the start-URL profile.
    let f = Fixture::sso();
    f.cache(MY_SSO_KEY, VALID);
    let o = run(f.cmd(), &["legacy"]);
    assert_ok(&o, "export AWS_PROFILE='legacy'\n");
    assert!(stderr(&o).contains("warning"), "{}", stderr(&o));
}

#[test]
fn login_on_non_sso_profile_runs_aws() {
    let f = Fixture::sso();
    let o = run(f.cmd(), &["--login", "dev"]);
    assert_ok(&o, "export AWS_PROFILE='dev'\n");
    assert_eq!(f.aws_called().as_deref(), Some("sso login --profile dev\n"));
}

// ---- exec ----

#[test]
fn exec_sets_profile_and_returns_child_code() {
    let f = Fixture::std();
    let o = run(
        f.cmd(),
        &[
            "exec",
            "dev",
            "--",
            "sh",
            "-c",
            r#"printf %s "$AWS_PROFILE"; exit 7"#,
        ],
    );
    assert_eq!(o.status.code(), Some(7), "{}", stderr(&o));
    assert_eq!(stdout(&o), "dev");
}

#[test]
fn exec_missing_command_127() {
    let f = Fixture::std();
    let o = run(
        f.cmd(),
        &["exec", "dev", "--", "aps-definitely-missing-cmd"],
    );
    assert_fails(&o, 127);
    assert!(
        stderr(&o).contains("aps-definitely-missing-cmd"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn exec_not_executable_126() {
    let f = Fixture::std();
    let script = f.home().join("not-exec.sh");
    fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();
    let o = run(f.cmd(), &["exec", "dev", "--", script.to_str().unwrap()]);
    assert_fails(&o, 126);
}

#[test]
fn exec_requires_command_exits_2() {
    let f = Fixture::std();
    assert_fails(&run(f.cmd(), &["exec", "dev"]), 2);
}

#[test]
fn exec_unknown_profile_exits_2() {
    let f = Fixture::std();
    let o = run(f.cmd(), &["exec", "nope", "--", "true"]);
    assert_fails(&o, 2);
    assert!(stderr(&o).contains("nope"), "{}", stderr(&o));
}

#[test]
fn exec_clear_env_removes_static_creds() {
    let f = Fixture::std();
    let script = r#"printf %s "${AWS_ACCESS_KEY_ID-none}""#;
    let mut c = f.cmd();
    c.env("AWS_ACCESS_KEY_ID", "AKIAEXAMPLE");
    let o = run(c, &["--clear-env", "exec", "dev", "--", "sh", "-c", script]);
    assert_ok(&o, "none");
    let mut c = f.cmd();
    c.env("AWS_ACCESS_KEY_ID", "AKIAEXAMPLE");
    let o = run(c, &["exec", "dev", "--", "sh", "-c", script]);
    assert_ok(&o, "AKIAEXAMPLE");
}

#[test]
fn exec_emit_mode_child_stdout_to_stderr() {
    let f = Fixture::std();
    let mut c = f.cmd();
    c.env("AWS_PROFILE_SWITCHER_SHELL", "zsh");
    let o = run(
        c,
        &[
            "exec",
            "dev",
            "--",
            "sh",
            "-c",
            r#"printf 'child:%s:%s' "$AWS_PROFILE" "${AWS_PROFILE_SWITCHER_SHELL-unset}""#,
        ],
    );
    assert_ok(&o, "");
    assert!(stderr(&o).contains("child:dev:unset"), "{}", stderr(&o));
}

// ---- contract sweep ----

/// (fixture, extra env, args, expected exit code)
type Case<'a> = (&'a Fixture, Vec<(&'a str, &'a str)>, Vec<&'a str>, i32);

#[test]
fn errors_never_write_stdout() {
    let std = Fixture::std();
    let empty = Fixture::new("", "");
    let sso = Fixture::sso();
    let cases: Vec<Case> = vec![
        (&std, vec![], vec![], 2),
        (&std, vec![], vec!["d"], 2),
        (&std, vec![], vec!["zzz"], 2),
        (&std, vec![], vec!["dev", "ls"], 2),
        (&std, vec![], vec!["--bogus"], 2),
        (
            &std,
            vec![("AWS_PROFILE_SWITCHER_SHELL", "zsh")],
            vec!["--bogus"],
            2,
        ),
        (
            &std,
            vec![("AWS_PROFILE_SWITCHER_SHELL", "tcsh")],
            vec!["dev"],
            2,
        ),
        (&std, vec![], vec!["current"], 1),
        (&std, vec![], vec!["init", "posix"], 2),
        (&std, vec![], vec!["init", "zsh", "--name", "1x"], 2),
        (&std, vec![], vec!["exec", "dev"], 2),
        (&std, vec![], vec!["exec", "nope", "--", "true"], 2),
        (
            &std,
            vec![],
            vec!["exec", "dev", "--", "aps-definitely-missing-cmd"],
            127,
        ),
        (&empty, vec![], vec!["dev"], 2),
        (&empty, vec![], vec!["ls"], 2),
        (&sso, vec![("FAKE_AWS_RC", "1")], vec!["--login", "s"], 2),
        (
            &sso,
            vec![("PATH", "/usr/bin:/bin")],
            vec!["--login", "s"],
            2,
        ),
        (&std, vec![], vec!["--login", "--no-sso", "dev"], 2),
    ];
    for (f, env, args, code) in cases {
        let mut c = f.cmd();
        for (k, v) in &env {
            c.env(k, v);
        }
        let o = run(c, &args);
        assert_eq!(
            o.status.code(),
            Some(code),
            "{args:?} {env:?}: {}",
            stderr(&o)
        );
        assert!(
            o.stdout.is_empty(),
            "{args:?} {env:?} wrote stdout: {:?}",
            stdout(&o)
        );
    }
}
