//! Interactive prompts. inquire draws on stderr, so nothing here touches stdout.

use std::io::{self, IsTerminal};

use inquire::{Confirm, InquireError, Select};

use crate::error::AppError;

/// Everything the picker needs, built by pure code so it is unit-testable.
#[derive(Debug, PartialEq, Eq)]
pub struct SelectRequest {
    pub items: Vec<String>,
    pub cursor: usize,
    pub filter: Option<String>,
}

/// With a filter the cursor starts at 0 (the filtered list is re-ordered);
/// otherwise it starts on `current` when that is in the list.
pub fn build_select(names: &[&str], current: Option<&str>, filter: Option<&str>) -> SelectRequest {
    let cursor = match (filter, current) {
        (None, Some(cur)) => names.iter().position(|n| *n == cur).unwrap_or(0),
        _ => 0,
    };
    SelectRequest {
        items: names.iter().map(|n| n.to_string()).collect(),
        cursor,
        filter: filter.map(str::to_string),
    }
}

/// Esc and Ctrl-C are a cancel (exit 1), never an error message on stdout.
fn map_inquire(e: InquireError) -> AppError {
    match e {
        InquireError::OperationCanceled | InquireError::OperationInterrupted => AppError::Cancelled,
        InquireError::NotTTY => AppError::NotATerminal,
        other => AppError::Prompt(other.to_string()),
    }
}

/// The picker and the confirm prompt need an interactive stderr. Checking up
/// front avoids inquire's redraw loop when stderr is a file or pipe.
pub fn ensure_terminal() -> Result<(), AppError> {
    if io::stderr().is_terminal() {
        Ok(())
    } else {
        Err(AppError::NotATerminal)
    }
}

pub fn pick(req: &SelectRequest) -> Result<String, AppError> {
    ensure_terminal()?;
    let mut select =
        Select::new("AWS profile:", req.items.clone()).with_starting_cursor(req.cursor);
    if let Some(f) = req.filter.as_deref() {
        select = select.with_starting_filter_input(f);
    }
    select.prompt().map_err(map_inquire)
}

pub fn confirm(message: &str) -> Result<bool, AppError> {
    ensure_terminal()?;
    Confirm::new(message)
        .with_default(true)
        .prompt()
        .map_err(map_inquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: &[&str] = &["a", "b", "c"];

    #[test]
    fn cursor_starts_on_current_profile() {
        let r = build_select(NAMES, Some("b"), None);
        assert_eq!(
            r,
            SelectRequest {
                items: vec!["a".into(), "b".into(), "c".into()],
                cursor: 1,
                filter: None,
            }
        );
    }

    #[test]
    fn cursor_defaults_0_when_current_missing() {
        assert_eq!(build_select(NAMES, Some("zzz"), None).cursor, 0);
        assert_eq!(build_select(NAMES, None, None).cursor, 0);
    }

    #[test]
    fn filter_sets_cursor_0() {
        let r = build_select(NAMES, Some("c"), Some("x"));
        assert_eq!(r.cursor, 0);
        assert_eq!(r.filter.as_deref(), Some("x"));
    }

    #[test]
    fn inquire_errors_map() {
        assert!(matches!(
            map_inquire(InquireError::OperationCanceled),
            AppError::Cancelled
        ));
        assert!(matches!(
            map_inquire(InquireError::OperationInterrupted),
            AppError::Cancelled
        ));
        assert!(matches!(
            map_inquire(InquireError::NotTTY),
            AppError::NotATerminal
        ));
        assert!(matches!(
            map_inquire(InquireError::IO(io::Error::other("x"))),
            AppError::Prompt(_)
        ));
        assert!(matches!(
            map_inquire(InquireError::InvalidConfiguration("x".into())),
            AppError::Prompt(_)
        ));
    }
}
