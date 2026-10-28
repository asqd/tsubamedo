//! The language of what the core says: Russian or English, one for the whole process. The app
//! sets it from its settings, the CLI from the environment.
//!
//! Every message is written in both, side by side, so neither drifts from the other:
//!
//! ```ignore
//! return Err(tr!("в ответе нет {path}", "{path} is not in the response"));
//! ```

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

thread_local! {
    static THIS_THREAD: Cell<Option<bool>> = const { Cell::new(None) };
}

pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

pub fn english() -> bool {
    THIS_THREAD.with(Cell::get).unwrap_or_else(|| ENGLISH.load(Ordering::Relaxed))
}

/// `f` with this thread in English (or Russian), the rest of the process as it is: for tests,
/// which run side by side.
pub fn in_language<T>(english: bool, f: impl FnOnce() -> T) -> T {
    let before = THIS_THREAD.with(|c| c.replace(Some(english)));
    let out = f();
    THIS_THREAD.with(|c| c.set(before));
    out
}

/// English unless the environment asks for Russian: `TSUBAMEDO_LANG=ru|en`, else the first of
/// `LC_ALL`, `LC_MESSAGES`, `LANG` that is set.
pub fn english_from_env() -> bool {
    if let Ok(lang) = std::env::var("TSUBAMEDO_LANG") {
        return !lang.trim().to_lowercase().starts_with("ru");
    }
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()));
    !locale.is_some_and(|l| l.to_lowercase().starts_with("ru"))
}

/// The message in the process's language, formatted: `tr!("в ответе нет {path}", "{path} is not
/// in the response")`, with arguments as in `format!`. Always a `String`, so a literal brace is
/// written `{{`.
#[macro_export]
macro_rules! tr {
    ($ru:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::lang::english() { format!($en $(, $arg)*) } else { format!($ru $(, $arg)*) }
    };
}

/// The same for a fixed text where a `&'static str` is needed; nothing is formatted.
#[macro_export]
macro_rules! tr_static {
    ($ru:literal, $en:literal $(,)?) => {
        if $crate::lang::english() { $en } else { $ru }
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn both_languages() {
        let path = "$.id";
        assert_eq!(tr!("в ответе нет {path}", "{path} is not in the response"), "в ответе нет $.id");
        assert_eq!(tr!("{} из {}", "{} of {}", 1, 2), "1 из 2");
        assert_eq!(tr_static!("да", "yes"), "да");
        assert_eq!(super::in_language(true, || tr!("в ответе нет {path}", "{path} is not in the response")), "$.id is not in the response");
        assert_eq!(tr_static!("да", "yes"), "да", "only for the closure");
    }
}
