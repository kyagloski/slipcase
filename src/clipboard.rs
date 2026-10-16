//! System clipboard for the text fields. One clipboard handle lives for the whole run: on
//! X11 and Wayland the copied text is served by it, so it must outlive the copy.

use std::sync::{Mutex, OnceLock};

static CLIPBOARD: OnceLock<Option<Mutex<arboard::Clipboard>>> = OnceLock::new();

fn with<R>(f: impl FnOnce(&mut arboard::Clipboard) -> Option<R>) -> Option<R> {
    let c = CLIPBOARD.get_or_init(|| arboard::Clipboard::new().ok().map(Mutex::new)).as_ref()?;
    let mut c = c.lock().ok()?;
    f(&mut c)
}

/// Clipboard text on one line (line breaks become spaces), or None if there is none.
pub fn paste() -> Option<String> {
    let t = with(|c| c.get_text().ok())?;
    let one_line: String = t.split(['\r', '\n']).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    (!one_line.is_empty()).then_some(one_line)
}

pub fn copy(text: &str) {
    let _ = with(|c| c.set_text(text.to_string()).ok());
}

#[cfg(test)]
mod tests {
    /// Needs a desktop session; only reads, so whatever is on the clipboard is left alone.
    #[test]
    #[ignore]
    fn reads_clipboard() {
        let ok = super::with(|c| Some(c.get_text().map(|t| t.len()).unwrap_or(0)));
        eprintln!("clipboard reachable: {}, text length: {:?}", ok.is_some(), ok);
        assert!(ok.is_some());
    }
}
