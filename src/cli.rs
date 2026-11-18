use std::ffi::OsString;

use clap::{Parser, Subcommand, ValueEnum};

use crate::shell::Shell;

/// Switch AWS_PROFILE in your current shell.
#[derive(Parser, Debug)]
#[command(name = "aws-profile-switcher", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,

    /// Profile to switch to (use `-- <name>` if it clashes with a subcommand)
    pub profile: Option<String>,

    /// Shell dialect to print [default: $AWS_PROFILE_SWITCHER_SHELL, else posix]
    #[arg(long, global = true, value_enum)]
    pub shell: Option<Shell>,

    /// Always run `aws sso login` before switching
    #[arg(long, global = true, conflicts_with = "no_sso")]
    pub login: bool,

    /// Skip the SSO token check
    #[arg(long, global = true)]
    pub no_sso: bool,

    /// Also unset AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY and AWS_SESSION_TOKEN
    #[arg(long, global = true)]
    pub clear_env: bool,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Print the shell function to add to your rc file
    Init {
        #[arg(value_enum, value_name = "SHELL")]
        target: InitShell,
        /// Name of the shell function
        #[arg(long, default_value = "awsprofile")]
        name: String,
    },
    /// Unset AWS_PROFILE
    Unset,
    /// Print the current AWS_PROFILE
    Current,
    /// List profiles
    Ls,
    /// Run a command with AWS_PROFILE set
    Exec {
        profile: String,
        #[arg(last = true, required = true, value_name = "COMMAND")]
        command: Vec<OsString>,
    },
}

/// Shells `init` has a function template for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum InitShell {
    Bash,
    Zsh,
    Fish,
}

impl From<InitShell> for Shell {
    fn from(s: InitShell) -> Shell {
        match s {
            InitShell::Bash => Shell::Bash,
            InitShell::Zsh => Shell::Zsh,
            InitShell::Fish => Shell::Fish,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }

    #[test]
    fn debug_assert() {
        Cli::command().debug_assert();
    }

    #[test]
    fn clear_env_before_unset_is_unset() {
        let c = parse(&["bin", "--clear-env", "unset"]).unwrap();
        assert_eq!(c.cmd, Some(Cmd::Unset));
        assert_eq!(c.profile, None);
        assert!(c.clear_env);
    }

    #[test]
    fn login_before_ls_is_ls() {
        let c = parse(&["bin", "--login", "ls"]).unwrap();
        assert_eq!(c.cmd, Some(Cmd::Ls));
        assert!(c.login);
    }

    #[test]
    fn double_dash_forces_profile() {
        let c = parse(&["bin", "--", "unset"]).unwrap();
        assert_eq!(c.profile.as_deref(), Some("unset"));
        assert_eq!(c.cmd, None);
    }

    #[test]
    fn profile_and_subcommand_detected() {
        // Either clap rejects it, or both fields are set and dispatch rejects it.
        match parse(&["bin", "dev", "ls"]) {
            Ok(c) => assert!(c.profile.is_some() && c.cmd.is_some(), "{c:?}"),
            Err(e) => assert_eq!(e.exit_code(), 2, "{e}"),
        }
    }

    #[test]
    fn exec_parses() {
        let c = parse(&["bin", "exec", "dev", "--", "sh", "-c", "x"]).unwrap();
        assert_eq!(
            c.cmd,
            Some(Cmd::Exec {
                profile: "dev".into(),
                command: vec!["sh".into(), "-c".into(), "x".into()],
            })
        );
    }

    #[test]
    fn exec_requires_command() {
        assert!(parse(&["bin", "exec", "dev"]).is_err());
        assert!(parse(&["bin", "exec", "dev", "--"]).is_err());
    }

    #[test]
    fn init_name_flag() {
        let c = parse(&["bin", "init", "zsh", "--name", "j"]).unwrap();
        assert_eq!(
            c.cmd,
            Some(Cmd::Init {
                target: InitShell::Zsh,
                name: "j".into()
            })
        );
        let c = parse(&["bin", "init", "fish"]).unwrap();
        assert_eq!(
            c.cmd,
            Some(Cmd::Init {
                target: InitShell::Fish,
                name: "awsprofile".into()
            })
        );
        assert_eq!(Shell::from(InitShell::Fish), Shell::Fish);
        assert_eq!(Shell::from(InitShell::Bash), Shell::Bash);
        assert_eq!(Shell::from(InitShell::Zsh), Shell::Zsh);
    }

    #[test]
    fn init_posix_rejected() {
        assert!(parse(&["bin", "init", "posix"]).is_err());
    }

    #[test]
    fn login_conflicts_no_sso() {
        assert!(parse(&["bin", "--login", "--no-sso", "dev"]).is_err());
    }

    #[test]
    fn shell_flag_global() {
        let c = parse(&["bin", "unset", "--shell", "fish"]).unwrap();
        assert_eq!(c.shell, Some(Shell::Fish));
        let c = parse(&["bin", "--shell", "posix", "dev"]).unwrap();
        assert_eq!(c.shell, Some(Shell::Posix));
        assert_eq!(c.profile.as_deref(), Some("dev"));
    }
}
