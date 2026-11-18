use std::io;
use std::path::PathBuf;

/// Every failure the tool can report. `exit_code()` maps each to the
/// documented process exit code (0 is never an error).
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("cancelled")]
    Cancelled,
    #[error("AWS_PROFILE is not set")]
    NoCurrentProfile,
    #[error("{0}")]
    Usage(String),
    #[error(
        "no AWS profiles found (looked in {} and {})",
        config.display(),
        credentials.display()
    )]
    NoProfiles {
        config: PathBuf,
        credentials: PathBuf,
    },
    #[error("unknown profile: {0}")]
    UnknownProfile(String),
    #[error(
        "not a terminal: the picker needs an interactive terminal on stderr (pass a profile name instead)"
    )]
    NotATerminal,
    #[error("cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("HOME is not set and no AWS_CONFIG_FILE/AWS_SHARED_CREDENTIALS_FILE override is given")]
    NoHome,
    #[error("aws sso login failed: {0}")]
    SsoLogin(String),
    #[error("prompt failed: {0}")]
    Prompt(String),
    #[error("{0}")]
    ExecNotFound(String),
    #[error("{0}")]
    ExecFailed(String),
}

impl AppError {
    pub fn exit_code(&self) -> u8 {
        match self {
            AppError::Cancelled | AppError::NoCurrentProfile => 1,
            AppError::ExecNotFound(_) => 127,
            AppError::ExecFailed(_) => 126,
            AppError::Usage(_)
            | AppError::NoProfiles { .. }
            | AppError::UnknownProfile(_)
            | AppError::NotATerminal
            | AppError::Read { .. }
            | AppError::NoHome
            | AppError::SsoLogin(_)
            | AppError::Prompt(_) => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_mapping() {
        let cases: Vec<(AppError, u8)> = vec![
            (AppError::Cancelled, 1),
            (AppError::NoCurrentProfile, 1),
            (AppError::Usage("u".into()), 2),
            (
                AppError::NoProfiles {
                    config: "c".into(),
                    credentials: "k".into(),
                },
                2,
            ),
            (AppError::UnknownProfile("x".into()), 2),
            (AppError::NotATerminal, 2),
            (
                AppError::Read {
                    path: "p".into(),
                    source: io::Error::other("x"),
                },
                2,
            ),
            (AppError::NoHome, 2),
            (AppError::SsoLogin("x".into()), 2),
            (AppError::Prompt("x".into()), 2),
            (AppError::ExecNotFound("x".into()), 127),
            (AppError::ExecFailed("x".into()), 126),
        ];
        for (err, code) in cases {
            assert_eq!(err.exit_code(), code, "{err:?}");
        }
    }

    #[test]
    fn no_profiles_message_names_both_paths() {
        let e = AppError::NoProfiles {
            config: "/h/.aws/config".into(),
            credentials: "/h/.aws/credentials".into(),
        };
        let s = e.to_string();
        assert!(s.contains("no AWS profiles found"), "{s}");
        assert!(s.contains("/h/.aws/config"), "{s}");
        assert!(s.contains("/h/.aws/credentials"), "{s}");
    }
}
