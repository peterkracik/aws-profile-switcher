#![deny(clippy::print_stdout)]

mod cli;
mod env;
mod error;
mod exec;
mod picker;
mod profiles;
mod shell;
mod sso;

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::time::SystemTime;

use clap::Parser;

use crate::cli::{Cli, Cmd};
use crate::env::Env;
use crate::error::AppError;
use crate::profiles::{Profile, Resolution};
use crate::shell::{Shell, ShellScript};
use crate::sso::SsoMode;

/// What a successful run produces. Only `write_output` turns it into bytes.
enum Output {
    /// Shell code for the caller to eval: always stdout.
    Script(ShellScript),
    /// Text for a human (`ls`, `current`, `init`, help). Goes to stderr in
    /// eval mode so the calling shell function never evals it.
    Data { text: String, eval_mode: bool },
}

fn main() -> ExitCode {
    let env = Env::from_process();
    let result = run(&env, std::env::args_os().collect(), SystemTime::now());
    match result.map(write_output) {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(e)) => {
            eprintln!("aws-profile-switcher: cannot write output: {e}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("aws-profile-switcher: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

/// The single place that writes stdout.
#[allow(clippy::print_stdout)]
fn write_output(out: Output) -> io::Result<()> {
    match out {
        Output::Script(script) => io::stdout().write_all(script.as_str().as_bytes()),
        Output::Data {
            text,
            eval_mode: false,
        } => io::stdout().write_all(text.as_bytes()),
        Output::Data {
            text,
            eval_mode: true,
        } => io::stderr().write_all(text.as_bytes()),
    }
}

/// `--shell` given anywhere before `--`. Used only to route clap's help and
/// version text when parsing stopped before `--shell` was read.
fn has_shell_flag(args: &[OsString]) -> bool {
    args.iter()
        .skip(1)
        .take_while(|a| *a != "--")
        .any(|a| a == "--shell" || a.as_encoded_bytes().starts_with(b"--shell="))
}

fn run(env: &Env, args: Vec<OsString>, now: SystemTime) -> Result<Output, AppError> {
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        // Help and version: exit 0; stderr if stdout may be eval'd.
        Err(e) if !e.use_stderr() => {
            return Ok(Output::Data {
                text: e.render().to_string(),
                eval_mode: env.switcher_shell.is_some() || has_shell_flag(&args),
            });
        }
        Err(e) => return Err(AppError::Usage(e.render().to_string().trim_end().into())),
    };

    // `--shell` beats the env var; either one means stdout will be eval'd.
    let shell = match cli.shell {
        Some(s) => Some(s),
        None => match env.switcher_shell.as_deref() {
            Some(v) => Shell::parse_env(v)?,
            None => None,
        },
    };
    let eval_mode = shell.is_some();
    let shell = shell.unwrap_or(Shell::Posix);
    let data = |text: String| Output::Data { text, eval_mode };

    if let (Some(p), Some(_)) = (&cli.profile, &cli.cmd) {
        return Err(AppError::Usage(format!(
            "a profile ({p:?}) cannot be combined with a subcommand; \
             use `-- <name>` for a profile named like a subcommand"
        )));
    }
    let sso_mode = if cli.login {
        SsoMode::Force
    } else if cli.no_sso {
        SsoMode::Skip
    } else {
        SsoMode::Ask
    };
    let tty = io::stderr().is_terminal();

    match cli.cmd {
        Some(Cmd::Init { target, name }) => {
            let exe = std::env::current_exe()
                .and_then(std::path::absolute)
                .map_err(|e| AppError::Usage(format!("cannot locate this binary: {e}")))?;
            let exe = shell::stable_exe(&exe, env.path.as_deref());
            Ok(data(shell::init_script(target.into(), &name, &exe)?))
        }
        Some(Cmd::Ls) => {
            let mut text = String::new();
            for p in profiles::load(env)? {
                text.push_str(&p.name);
                text.push('\n');
            }
            Ok(data(text))
        }
        Some(Cmd::Current) => match &env.aws_profile {
            Some(p) => Ok(data(format!("{p}\n"))),
            None => Err(AppError::NoCurrentProfile),
        },
        Some(Cmd::Unset) => Ok(Output::Script(shell::emit_unset(shell, cli.clear_env))),
        Some(Cmd::Exec { profile, command }) => {
            let profiles = profiles::load(env)?;
            let chosen = profiles
                .iter()
                .find(|p| p.name == profile)
                .ok_or(AppError::UnknownProfile(profile))?;
            warn_static_creds(env, cli.clear_env);
            sso::ensure_session(chosen, env, sso_mode, now, tty)?;
            Err(exec::run(&chosen.name, &command, cli.clear_env, eval_mode))
        }
        None => {
            let profiles = profiles::load(env)?;
            let chosen = choose(&profiles, cli.profile.as_deref(), env, tty)?;
            warn_static_creds(env, cli.clear_env);
            sso::ensure_session(chosen, env, sso_mode, now, tty)?;
            Ok(Output::Script(shell::emit_switch(
                shell,
                &chosen.name,
                cli.clear_env,
            )))
        }
    }
}

/// Resolves the profile argument, or opens the picker.
fn choose<'a>(
    profiles: &'a [Profile],
    query: Option<&str>,
    env: &Env,
    tty: bool,
) -> Result<&'a Profile, AppError> {
    let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
    let filter = match query.map(|q| (q, profiles::resolve(q, &names))) {
        None => None,
        Some((_, Resolution::Exact(i))) => return Ok(&profiles[i]),
        Some((q, Resolution::Unique(i))) => {
            eprintln!(
                "aws-profile-switcher: {q:?} matched profile '{}'",
                profiles[i].name
            );
            return Ok(&profiles[i]);
        }
        Some((q, Resolution::Pick { filter })) => {
            if !tty {
                let q = q.to_lowercase();
                return Err(if names.iter().any(|n| n.to_lowercase().contains(&q)) {
                    AppError::NotATerminal
                } else {
                    AppError::UnknownProfile(filter)
                });
            }
            Some(filter)
        }
    };
    let request = picker::build_select(&names, env.aws_profile.as_deref(), filter.as_deref());
    let picked = picker::pick(&request)?;
    profiles
        .iter()
        .find(|p| p.name == picked)
        .ok_or(AppError::UnknownProfile(picked))
}

/// Static credentials in the environment win over AWS_PROFILE for the AWS
/// CLI and SDKs, so the switch would silently not take effect.
fn warn_static_creds(env: &Env, clear_env: bool) {
    if !clear_env && !env.static_creds.is_empty() {
        eprintln!(
            "aws-profile-switcher: warning: {} set in the environment and override AWS_PROFILE; \
             pass --clear-env to unset them",
            env.static_creds.join(", ")
        );
    }
}
