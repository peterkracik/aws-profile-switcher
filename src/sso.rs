//! SSO token-cache check and `aws sso login`.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::SystemTime;

use sha1::{Digest, Sha1};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::env::{Env, SWITCHER_SHELL_VAR};
use crate::error::AppError;
use crate::picker;
use crate::profiles::{Profile, SsoRef};

/// From the CLI flags: `--login` = Force, `--no-sso` = Skip, else Ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsoMode {
    Ask,
    Force,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenStatus {
    Valid,
    Expired,
    Missing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsoAction {
    Proceed,
    Login,
    Ask,
    Warn,
}

/// The cache file name (without `.json`): hex SHA-1 of the session name or
/// the legacy start URL, as the AWS CLI computes it.
pub fn cache_key(r: &SsoRef) -> String {
    let input = match r {
        SsoRef::Session(s) | SsoRef::StartUrl(s) => s,
    };
    Sha1::digest(input.as_bytes())
        .iter()
        .fold(String::with_capacity(40), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        })
}

/// RFC 3339, plus the legacy `…UTC` suffix older CLIs wrote.
fn parse_expires_at(s: &str) -> Option<SystemTime> {
    let normalized = match s.strip_suffix("UTC") {
        Some(base) => format!("{base}Z"),
        None => s.to_string(),
    };
    OffsetDateTime::parse(&normalized, &Rfc3339)
        .ok()
        .map(SystemTime::from)
}

/// Unreadable or malformed cache entries count as `Expired` (fail safe: ask).
pub fn token_status(cache_dir: &Path, r: &SsoRef, now: SystemTime) -> TokenStatus {
    let path = cache_dir.join(format!("{}.json", cache_key(r)));
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return TokenStatus::Missing,
        Err(_) => return TokenStatus::Expired,
    };
    let expires = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| v.get("expiresAt")?.as_str().and_then(parse_expires_at));
    match expires {
        Some(t) if t > now => TokenStatus::Valid,
        _ => TokenStatus::Expired,
    }
}

pub fn decide(mode: SsoMode, status: TokenStatus, tty: bool) -> SsoAction {
    match (mode, status) {
        (SsoMode::Skip, _) | (SsoMode::Ask, TokenStatus::Valid) => SsoAction::Proceed,
        (SsoMode::Force, _) => SsoAction::Login,
        (SsoMode::Ask, _) if tty => SsoAction::Ask,
        (SsoMode::Ask, _) => SsoAction::Warn,
    }
}

fn describe(status: TokenStatus) -> &'static str {
    match status {
        TokenStatus::Missing => "has no cached SSO token",
        _ => "has an expired SSO token",
    }
}

/// Makes sure the profile's SSO session is usable, per `mode`. Declining the
/// prompt, or having no terminal, switches anyway with a warning.
/// `--login` (Force) logs in even for non-SSO profiles, which covers
/// `source_profile` chains by hand.
pub fn ensure_session(
    profile: &Profile,
    env: &Env,
    mode: SsoMode,
    now: SystemTime,
    tty: bool,
) -> Result<(), AppError> {
    let status = match (mode, &profile.sso) {
        (SsoMode::Force, _) => TokenStatus::Missing,
        (SsoMode::Skip, _) | (SsoMode::Ask, None) => return Ok(()),
        (SsoMode::Ask, Some(r)) => token_status(&env.sso_cache_dir()?, r, now),
    };
    let name = &profile.name;
    let warn = || {
        eprintln!(
            "aws-profile-switcher: warning: profile '{name}' {}; switching anyway \
             (run `aws sso login --profile {name}` or pass --login)",
            describe(status)
        );
    };
    match decide(mode, status, tty) {
        SsoAction::Proceed => Ok(()),
        SsoAction::Login => login(name),
        SsoAction::Warn => {
            warn();
            Ok(())
        }
        SsoAction::Ask => {
            let question = format!(
                "Profile '{name}' {}. Run `aws sso login` now? \
                 (the session may refresh automatically; answer n to switch anyway)",
                describe(status)
            );
            if picker::confirm(&question)? {
                login(name)
            } else {
                warn();
                Ok(())
            }
        }
    }
}

/// `aws sso login`, with its stdout sent to our stderr so the URL and code
/// can never be eval'd. Its stderr is inherited, so aws errors stay visible.
pub fn login(profile: &str) -> Result<(), AppError> {
    let status = Command::new("aws")
        .args(["sso", "login", "--profile", profile])
        .env_remove(SWITCHER_SHELL_VAR)
        .stdout(Stdio::from(io::stderr()))
        .status()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => AppError::SsoLogin("aws CLI not found on PATH".into()),
            _ => AppError::SsoLogin(format!("cannot run aws: {e}")),
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(AppError::SsoLogin(format!("aws {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> SystemTime {
        parse_expires_at(s).expect(s)
    }

    #[test]
    fn cache_key_vectors() {
        assert_eq!(
            cache_key(&SsoRef::Session("abc".into())),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            cache_key(&SsoRef::Session("my-sso".into())),
            "0ad374308c5a4e22f723adf10145eafad7c4031c"
        );
        assert_eq!(
            cache_key(&SsoRef::StartUrl(
                "https://example.awsapps.com/start".into()
            )),
            "e8be5486177c5b5392bd9aa76563515b29358e6e"
        );
    }

    #[test]
    fn token_status_cases() {
        let dir = tempfile::tempdir().unwrap();
        let r = SsoRef::Session("my-sso".into());
        let file = dir.path().join(format!("{}.json", cache_key(&r)));
        let now = at("2026-11-18T12:00:00Z");
        let cases: &[(&str, TokenStatus)] = &[
            (
                r#"{"expiresAt":"2026-11-18T13:00:00Z"}"#,
                TokenStatus::Valid,
            ),
            (
                r#"{"expiresAt":"2026-11-18T11:00:00Z"}"#,
                TokenStatus::Expired,
            ),
            (
                r#"{"expiresAt":"2026-11-18T12:00:00Z"}"#,
                TokenStatus::Expired,
            ),
            (
                r#"{"expiresAt":"2026-11-18T13:00:00UTC"}"#,
                TokenStatus::Valid,
            ),
            (
                r#"{"expiresAt":"2026-11-18T13:00:00.123Z"}"#,
                TokenStatus::Valid,
            ),
            (
                r#"{"expiresAt":"2026-11-18T14:00:00+01:00"}"#,
                TokenStatus::Valid,
            ),
            ("{not json", TokenStatus::Expired),
            ("[]", TokenStatus::Expired),
            (r#"{"accessToken":"x"}"#, TokenStatus::Expired),
            (r#"{"expiresAt":1763467200}"#, TokenStatus::Expired),
            (r#"{"expiresAt":"tomorrow"}"#, TokenStatus::Expired),
        ];
        for (json, want) in cases {
            fs::write(&file, json).unwrap();
            assert_eq!(token_status(dir.path(), &r, now), *want, "{json}");
        }
        fs::remove_file(&file).unwrap();
        assert_eq!(token_status(dir.path(), &r, now), TokenStatus::Missing);
    }

    #[test]
    fn decide_table() {
        use SsoAction as A;
        use SsoMode::*;
        use TokenStatus::*;
        for tty in [true, false] {
            assert_eq!(decide(Ask, Valid, tty), A::Proceed);
            assert_eq!(decide(Skip, Valid, tty), A::Proceed);
            for st in [Valid, Expired, Missing] {
                assert_eq!(decide(Skip, st, tty), A::Proceed);
                assert_eq!(decide(Force, st, tty), A::Login);
            }
        }
        for st in [Expired, Missing] {
            assert_eq!(decide(Ask, st, true), A::Ask);
            assert_eq!(decide(Ask, st, false), A::Warn);
        }
    }
}
