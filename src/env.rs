use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::error::AppError;

/// The credential variables that override any profile when set.
pub const STATIC_CRED_VARS: [&str; 3] = [
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
];

/// Variable the shell function sets so the binary knows its stdout is eval'd.
pub const SWITCHER_SHELL_VAR: &str = "AWS_PROFILE_SWITCHER_SHELL";

/// Snapshot of every environment value the tool uses. Taken once in `main`;
/// everything else receives `&Env`, so tests never mutate the process env.
#[derive(Debug, Default, Clone)]
pub struct Env {
    pub home: Option<PathBuf>,
    pub aws_profile: Option<String>,
    pub config_file: Option<OsString>,
    pub credentials_file: Option<OsString>,
    pub switcher_shell: Option<String>,
    pub path: Option<OsString>,
    pub static_creds: Vec<&'static str>,
}

impl Env {
    /// The only place that reads the process environment.
    pub fn from_process() -> Self {
        Self::from_lookup(|k| std::env::var_os(k))
    }

    /// Builds the snapshot from any lookup function. Empty values count as unset.
    pub fn from_lookup(get: impl Fn(&str) -> Option<OsString>) -> Self {
        let get = |k: &str| get(k).filter(|v| !v.is_empty());
        Env {
            home: get("HOME").map(PathBuf::from),
            aws_profile: get("AWS_PROFILE").map(|v| v.to_string_lossy().into_owned()),
            config_file: get("AWS_CONFIG_FILE"),
            credentials_file: get("AWS_SHARED_CREDENTIALS_FILE"),
            switcher_shell: get(SWITCHER_SHELL_VAR).map(|v| v.to_string_lossy().into_owned()),
            path: get("PATH"),
            static_creds: STATIC_CRED_VARS
                .into_iter()
                .filter(|k| get(k).is_some())
                .collect(),
        }
    }

    fn home(&self) -> Result<&Path, AppError> {
        self.home.as_deref().ok_or(AppError::NoHome)
    }

    /// Expands a leading `~` or `~/` against HOME, like the AWS CLI does.
    fn expand(&self, value: &OsString) -> Result<PathBuf, AppError> {
        let path = Path::new(value);
        // Component-wise: `~` and `~/x` match, `~bob/x` does not.
        match path.strip_prefix("~") {
            Ok(rest) => Ok(self.home()?.join(rest)),
            Err(_) => Ok(path.to_path_buf()),
        }
    }

    /// `AWS_CONFIG_FILE` (with `~` expanded), else `$HOME/.aws/config`.
    pub fn config_path(&self) -> Result<PathBuf, AppError> {
        match &self.config_file {
            Some(v) => self.expand(v),
            None => Ok(self.home()?.join(".aws").join("config")),
        }
    }

    /// `AWS_SHARED_CREDENTIALS_FILE` (with `~` expanded), else `$HOME/.aws/credentials`.
    pub fn credentials_path(&self) -> Result<PathBuf, AppError> {
        match &self.credentials_file {
            Some(v) => self.expand(v),
            None => Ok(self.home()?.join(".aws").join("credentials")),
        }
    }

    /// `$HOME/.aws/sso/cache`.
    pub fn sso_cache_dir(&self) -> Result<PathBuf, AppError> {
        Ok(self.home()?.join(".aws").join("sso").join("cache"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_home() -> Env {
        Env {
            home: Some("/h".into()),
            ..Env::default()
        }
    }

    #[test]
    fn config_path_default() {
        assert_eq!(
            with_home().config_path().unwrap(),
            PathBuf::from("/h/.aws/config")
        );
        assert_eq!(
            with_home().credentials_path().unwrap(),
            PathBuf::from("/h/.aws/credentials")
        );
    }

    #[test]
    fn config_path_override_tilde() {
        let mut env = with_home();
        env.config_file = Some("~/x".into());
        assert_eq!(env.config_path().unwrap(), PathBuf::from("/h/x"));
        env.config_file = Some("~".into());
        assert_eq!(env.config_path().unwrap(), PathBuf::from("/h"));
        // `~user` is not expanded.
        env.config_file = Some("~bob/x".into());
        assert_eq!(env.config_path().unwrap(), PathBuf::from("~bob/x"));
    }

    #[test]
    fn credentials_path_override_absolute() {
        let env = Env {
            credentials_file: Some("/abs/creds".into()),
            ..Env::default()
        };
        assert_eq!(env.credentials_path().unwrap(), PathBuf::from("/abs/creds"));
    }

    #[test]
    fn sso_cache_dir() {
        assert_eq!(
            with_home().sso_cache_dir().unwrap(),
            PathBuf::from("/h/.aws/sso/cache")
        );
    }

    #[test]
    fn no_home_no_override_is_error() {
        let env = Env::default();
        assert!(matches!(env.config_path(), Err(AppError::NoHome)));
        assert!(matches!(env.credentials_path(), Err(AppError::NoHome)));
        assert!(matches!(env.sso_cache_dir(), Err(AppError::NoHome)));
        let tilde = Env {
            config_file: Some("~/x".into()),
            ..Env::default()
        };
        assert!(matches!(tilde.config_path(), Err(AppError::NoHome)));
    }

    #[test]
    fn static_creds_listed() {
        let env = Env::from_lookup(|k| match k {
            "AWS_ACCESS_KEY_ID" => Some("AKIA".into()),
            "AWS_SESSION_TOKEN" => Some("".into()),
            "AWS_PROFILE" => Some("dev".into()),
            "AWS_PROFILE_SWITCHER_SHELL" => Some("".into()),
            _ => None,
        });
        assert_eq!(env.static_creds, vec!["AWS_ACCESS_KEY_ID"]);
        assert_eq!(env.aws_profile.as_deref(), Some("dev"));
        assert_eq!(env.switcher_shell, None);
        assert_eq!(env.home, None);
        assert_eq!(env.path, None);
    }

    #[test]
    fn path_captured() {
        let env = Env::from_lookup(|k| (k == "PATH").then(|| "/a:/b".into()));
        assert_eq!(env.path, Some(OsString::from("/a:/b")));
    }
}
