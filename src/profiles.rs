use std::fs;
use std::io;
use std::path::Path;

use crate::env::Env;
use crate::error::AppError;

/// Where a profile's SSO token lives in the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsoRef {
    /// `sso_session = <name>`; the cache key is the session name.
    Session(String),
    /// Legacy `sso_start_url = <url>`; the cache key is the URL.
    StartUrl(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub sso: Option<SsoRef>,
}

/// One logical line: CR stripped; `None` for blank lines, comments and
/// indented sub-keys (which belong to a nested block such as `s3 =`).
fn significant(line: &str) -> Option<&str> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let line = line.trim_end();
    if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
        return None;
    }
    Some(line)
}

/// `[inner]` -> `inner` trimmed.
fn section(line: &str) -> Option<&str> {
    line.strip_prefix('[')?
        .rsplit_once(']')
        .map(|(inner, _)| inner.trim())
}

/// The profile name of a config-file section, or `None` for non-profile
/// sections and names botocore would not accept (an unquoted space).
fn config_profile_name(inner: &str) -> Option<String> {
    if inner == "default" {
        return Some(inner.to_string());
    }
    let rest = inner.strip_prefix("profile")?;
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let rest = rest.trim();
    for q in ['"', '\''] {
        if let Some(n) = rest.strip_prefix(q).and_then(|r| r.strip_suffix(q)) {
            return Some(n.to_string());
        }
    }
    if rest.is_empty() || rest.contains(char::is_whitespace) {
        return None;
    }
    Some(rest.to_string())
}

#[derive(Default)]
struct Draft {
    name: String,
    session: Option<String>,
    start_url: Option<String>,
}

pub fn parse_config(text: &str) -> Vec<Profile> {
    let mut drafts: Vec<Draft> = Vec::new();
    // Index into `drafts` of the section being read; `None` = not a profile.
    let mut current: Option<usize> = None;
    for line in text.split('\n').filter_map(significant) {
        if let Some(inner) = section(line) {
            current = config_profile_name(inner).map(|name| {
                drafts
                    .iter()
                    .position(|d| d.name == name)
                    .unwrap_or_else(|| {
                        drafts.push(Draft {
                            name,
                            ..Draft::default()
                        });
                        drafts.len() - 1
                    })
            });
            continue;
        }
        let (Some(i), Some((key, value))) = (current, line.split_once('=')) else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "sso_session" if !value.is_empty() => drafts[i].session = Some(value.into()),
            "sso_start_url" if !value.is_empty() => drafts[i].start_url = Some(value.into()),
            _ => {}
        }
    }
    drafts
        .into_iter()
        .map(|d| Profile {
            sso: d
                .session
                .map(SsoRef::Session)
                .or(d.start_url.map(SsoRef::StartUrl)),
            name: d.name,
        })
        .collect()
}

/// Section headers only; value lines are never inspected, so secrets in the
/// credentials file can't end up in any output.
pub fn parse_credentials_sections(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for inner in text.split('\n').filter_map(significant).filter_map(section) {
        if !inner.is_empty() && !out.iter().any(|n| n == inner) {
            out.push(inner.to_string());
        }
    }
    out
}

/// Reads a file; a missing file is empty, bad UTF-8 is replaced, not fatal.
fn read_lossy(path: &Path) -> Result<String, AppError> {
    match fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(source) => Err(AppError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// All profiles: config order first, then credentials-only names.
pub fn load(env: &Env) -> Result<Vec<Profile>, AppError> {
    let config = env.config_path()?;
    let credentials = env.credentials_path()?;
    let mut profiles = parse_config(&read_lossy(&config)?);
    for name in parse_credentials_sections(&read_lossy(&credentials)?) {
        if !profiles.iter().any(|p| p.name == name) {
            profiles.push(Profile { name, sso: None });
        }
    }
    if profiles.is_empty() {
        return Err(AppError::NoProfiles {
            config,
            credentials,
        });
    }
    Ok(profiles)
}

/// How a profile argument maps onto the known names.
#[derive(Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Exact (case-sensitive) name match.
    Exact(usize),
    /// Exactly one case-insensitive substring match: auto-selected.
    Unique(usize),
    /// Zero or several matches: open the picker pre-filtered.
    Pick { filter: String },
}

pub fn resolve(query: &str, names: &[&str]) -> Resolution {
    if let Some(i) = names.iter().position(|n| *n == query) {
        return Resolution::Exact(i);
    }
    let q = query.to_lowercase();
    let mut hits = names
        .iter()
        .enumerate()
        .filter(|(_, n)| n.to_lowercase().contains(&q))
        .map(|(i, _)| i);
    match (hits.next(), hits.next()) {
        (Some(i), None) => Resolution::Unique(i),
        _ => Resolution::Pick {
            filter: query.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(p: &[Profile]) -> Vec<&str> {
        p.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn config_headers() {
        let cases: &[(&str, &[&str])] = &[
            ("[default]\n", &["default"]),
            ("[profile  dev ]\n", &["dev"]),
            ("[profile \"a b\"]\n", &["a b"]),
            ("[profile 'q;echo PWNED']\n", &["q;echo PWNED"]),
            ("[profile x y]\n", &[]),
            ("[profile x;id]\n", &["x;id"]),
            ("[profile x$(id)]\n", &["x$(id)"]),
            ("[sso-session s]\nsso_start_url = u\n", &[]),
            ("[services x]\n", &[]),
            ("# [profile c1]\n; [profile c2]\n", &[]),
            ("[profile a]\r\nregion = x\r\n[profile b]\r\n", &["a", "b"]),
            ("[profile a]\ns3 =\n  [profile nested]\n", &["a"]),
            ("[profile a]\n[profile a]\n[default]\n", &["a", "default"]),
        ];
        for (text, want) in cases {
            assert_eq!(names(&parse_config(text)), *want, "input: {text:?}");
        }
    }

    #[test]
    fn config_sso_fields() {
        let text = "\
[profile s]
sso_session = my-sso
sso_account_id = 1
[profile legacy]
sso_start_url = https://example.awsapps.com/start
[profile both]
sso_start_url = https://u
sso_session = sess
[profile plain]
region = eu-west-1
  sso_session = nested-ignored
[sso-session my-sso]
sso_start_url = https://ignored
";
        let p = parse_config(text);
        assert_eq!(
            p,
            vec![
                Profile {
                    name: "s".into(),
                    sso: Some(SsoRef::Session("my-sso".into()))
                },
                Profile {
                    name: "legacy".into(),
                    sso: Some(SsoRef::StartUrl("https://example.awsapps.com/start".into()))
                },
                Profile {
                    name: "both".into(),
                    sso: Some(SsoRef::Session("sess".into()))
                },
                Profile {
                    name: "plain".into(),
                    sso: None
                },
            ]
        );
    }

    #[test]
    fn credentials_sections() {
        let text = "[default]\naws_access_key_id = A\n[y;echo PWNED4]\n# [c]\n[ spaced ]\n";
        assert_eq!(
            parse_credentials_sections(text),
            vec!["default", "y;echo PWNED4", "spaced"]
        );
    }

    #[test]
    fn credentials_values_never_leak() {
        let text = "[k]\naws_secret_access_key = SECRETVALUE\n[SECRETVALUE2\n";
        let out = parse_credentials_sections(text);
        assert_eq!(out, vec!["k"]);
        assert!(out.iter().all(|s| !s.contains("SECRETVALUE")));
    }

    fn env_for(dir: &Path) -> Env {
        Env {
            home: Some(dir.to_path_buf()),
            config_file: Some(dir.join("config").into()),
            credentials_file: Some(dir.join("credentials").into()),
            ..Env::default()
        }
    }

    #[test]
    fn load_merges_config_then_credentials() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("config"), "[profile b]\n[default]\n").unwrap();
        fs::write(d.path().join("credentials"), "[default]\n[c]\n[b]\n").unwrap();
        let p = load(&env_for(d.path())).unwrap();
        assert_eq!(names(&p), vec!["b", "default", "c"]);
    }

    #[test]
    fn load_missing_files_is_no_profiles() {
        let d = tempfile::tempdir().unwrap();
        assert!(matches!(
            load(&env_for(d.path())),
            Err(AppError::NoProfiles { .. })
        ));
    }

    #[test]
    fn load_invalid_utf8_does_not_panic() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("config"), b"[profile a\xff]\n[profile ok]\n").unwrap();
        let p = load(&env_for(d.path())).unwrap();
        assert!(names(&p).contains(&"ok"));
    }

    #[test]
    fn load_unreadable_path_is_read_error() {
        let d = tempfile::tempdir().unwrap();
        let mut env = env_for(d.path());
        env.config_file = Some(d.path().into()); // a directory
        match load(&env) {
            Err(AppError::Read { path, .. }) => assert_eq!(path, d.path()),
            other => panic!("expected Read error, got {other:?}"),
        }
    }

    mod resolve {
        use super::super::*;

        const NAMES: &[&str] = &["dev", "my-dev", "prod", "default"];

        #[test]
        fn exact() {
            assert_eq!(resolve("dev", NAMES), Resolution::Exact(0));
        }

        #[test]
        fn unique_ci_substring() {
            assert_eq!(resolve("PRO", NAMES), Resolution::Unique(2));
        }

        #[test]
        fn ambiguous() {
            assert_eq!(resolve("d", NAMES), Resolution::Pick { filter: "d".into() });
        }

        #[test]
        fn none() {
            assert_eq!(
                resolve("zzz", NAMES),
                Resolution::Pick {
                    filter: "zzz".into()
                }
            );
        }

        #[test]
        fn exact_beats_substring() {
            // "dev" is also a substring of "my-dev"; exact still wins.
            assert_eq!(resolve("my-dev", NAMES), Resolution::Exact(1));
            assert_eq!(resolve("dev", &["my-dev", "dev"]), Resolution::Exact(1));
        }
    }
}
