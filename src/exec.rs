//! `exec <profile> -- cmd…`: replace this process with `cmd`, AWS_PROFILE set.

use std::ffi::OsString;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use crate::env::{STATIC_CRED_VARS, SWITCHER_SHELL_VAR};
use crate::error::AppError;

/// Only returns on failure; on success the child replaces this process and
/// its exit code becomes ours. In eval mode the child's stdout goes to our
/// stderr so the calling shell function never evals it.
pub fn run(profile: &str, command: &[OsString], clear_env: bool, eval_mode: bool) -> AppError {
    let Some((program, args)) = command.split_first() else {
        return AppError::Usage("exec needs a command after `--`".into());
    };
    let mut cmd = Command::new(program);
    cmd.args(args)
        .env("AWS_PROFILE", profile)
        .env_remove(SWITCHER_SHELL_VAR);
    if clear_env {
        for var in STATIC_CRED_VARS {
            cmd.env_remove(var);
        }
    }
    if eval_mode {
        cmd.stdout(Stdio::from(io::stderr()));
    }
    let err = cmd.exec();
    let shown = program.to_string_lossy();
    match err.kind() {
        io::ErrorKind::NotFound => AppError::ExecNotFound(format!("{shown}: command not found")),
        _ => AppError::ExecFailed(format!("{shown}: cannot execute: {err}")),
    }
}
