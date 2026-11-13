//! The one Formatted PDF draft: the document being written, with its page and font settings, kept in the app's data
//! folder so it survives switching tools, closing the app and a crash. It never leaves this device.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::kit::pdf::formatted::FormattedDocument;
use crate::settings::write_json_atomically;

pub const DRAFT_FILE: &str = "formatted-pdf-draft.json";

/// Saves, loads and clears take turns, so a clear never races a save into leaving a stale draft behind.
static DRAFT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormattedDraft {
    pub document: FormattedDocument,
    /// When the draft was last saved, in milliseconds since the Unix epoch.
    pub saved_at: u64,
}

/// What loading found: the draft, if there is a usable one, and a problem to report when the stored draft could not
/// be used.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftLoad {
    pub draft: Option<FormattedDraft>,
    pub problem: Option<String>,
}

fn lock() -> std::sync::MutexGuard<'static, ()> { DRAFT_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) }

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

/// Replaces the draft with `document`, checked as saving the PDF checks it. A blank document is still saved; the view
/// clears the draft instead when nothing has been written.
pub fn save(path: &Path, document: FormattedDocument) -> Result<FormattedDraft, String> {
    document.check().map_err(|error| error.message)?;
    let draft = FormattedDraft { document, saved_at: now_millis() };
    let _guard = lock();
    write_json_atomically(path, &draft, "the draft")?;
    Ok(draft)
}

/// Reads the draft back. A draft that can't be read as one, or that saving the PDF would refuse, is reported and
/// removed so it can't fail again the next time the tool opens.
pub fn load(path: &Path) -> DraftLoad {
    let _guard = lock();
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return DraftLoad { draft: None, problem: None },
        Err(error) => return DraftLoad { draft: None, problem: Some(format!("Your draft couldn't be read: {error}")) },
    };
    let checked = serde_json::from_slice::<FormattedDraft>(&bytes)
        .map_err(|_| "Your saved draft was damaged, so it was cleared.".to_string())
        .and_then(|draft| match draft.document.check() {
            Ok(()) => Ok(draft),
            Err(error) => Err(format!("Your saved draft couldn't be opened, so it was cleared. {}", error.message)),
        });
    match checked {
        Ok(draft) => DraftLoad { draft: Some(draft), problem: None },
        Err(problem) => {
            let _ = remove(path);
            DraftLoad { draft: None, problem: Some(problem) }
        }
    }
}

/// Removes the draft. There being no draft is not an error.
pub fn clear(path: &Path) -> Result<(), String> {
    let _guard = lock();
    remove(path)
}

fn remove(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(format!("Couldn't clear the draft: {error}")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn sandbox(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("toolbox-formatted-draft-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn plain(text: &str) -> serde_json::Value { json!({ "text": text, "bold": false, "italic": false }) }

    /// Everything the editor keeps, including what saving the PDF leaves out: blank blocks, blank items and cells,
    /// and untrimmed text.
    fn document() -> serde_json::Value {
        json!({
            "title": "  Move-out checklist ",
            "author": "Sam Lee",
            "blocks": [
                { "kind": "heading1", "text": "Move-out checklist" },
                { "kind": "paragraph", "runs": [plain("Return the "), { "text": "keys", "bold": true, "italic": false }, { "text": " today.", "bold": false, "italic": true }] },
                { "kind": "paragraph", "runs": [] },
                { "kind": "bulletedList", "items": [{ "runs": [plain("Oven")] }, { "runs": [] }] },
                { "kind": "numberedList", "items": [{ "runs": [plain("Hob")] }] },
                { "kind": "table", "headerRow": true, "rows": [["Room", "", " Done "], ["", "", ""]] },
                { "kind": "image", "path": "/Users/sam/Pictures/floorplan.png" },
                { "kind": "pageBreak" },
                { "kind": "heading3", "text": "" },
            ],
            "page": { "pageSize": "letter", "orientation": "landscape", "margins": "wide", "header": "Flat 4", "footer": " ", "pageNumbers": true },
            "font": { "family": "Liberation Sans", "size": 12.5 },
        })
    }

    #[test]
    fn save_load_and_clear_round_trip_the_exact_document() {
        let root = sandbox("round-trip");
        let path = root.join("app-data").join(DRAFT_FILE);
        assert_eq!(load(&path), DraftLoad { draft: None, problem: None });

        let sent: FormattedDocument = serde_json::from_value(document()).unwrap();
        let saved = save(&path, sent.clone()).unwrap();
        assert_eq!(saved.document, sent);
        assert!(saved.saved_at > 0);
        assert!(!path.with_extension("json.tmp").exists());

        let loaded = load(&path);
        assert_eq!(loaded, DraftLoad { draft: Some(saved.clone()), problem: None });
        assert_eq!(serde_json::to_value(&loaded.draft.unwrap().document).unwrap(), document());

        let mut changed = sent.clone();
        changed.title = "Keys".into();
        save(&path, changed.clone()).unwrap();
        assert_eq!(load(&path).draft.unwrap().document, changed);

        clear(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(load(&path), DraftLoad { draft: None, problem: None });
        clear(&path).unwrap();
    }

    #[test]
    fn a_damaged_draft_is_reported_and_cleared() {
        let root = sandbox("damaged");
        let path = root.join(DRAFT_FILE);
        std::fs::write(&path, b"{\"document\": {\"title\": \"Move-out").unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.draft, None);
        assert_eq!(loaded.problem.as_deref(), Some("Your saved draft was damaged, so it was cleared."));
        assert!(!path.exists());
        assert_eq!(load(&path), DraftLoad { draft: None, problem: None });
    }

    #[test]
    fn a_draft_saving_would_refuse_is_reported_and_cleared_on_load() {
        let root = sandbox("refused");
        let path = root.join(DRAFT_FILE);
        let mut document = document();
        document["blocks"] = json!([{ "kind": "table", "headerRow": false, "rows": [["a", "b"], ["c"]] }]);
        std::fs::write(&path, serde_json::to_vec(&json!({ "document": document, "savedAt": 1 })).unwrap()).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.draft, None);
        assert!(loaded.problem.unwrap().contains("Every row of a table needs the same number of cells."));
        assert!(!path.exists());
    }

    #[test]
    fn save_refuses_a_document_the_pdf_check_refuses_and_keeps_the_last_draft() {
        let root = sandbox("refuse-save");
        let path = root.join(DRAFT_FILE);
        let good: FormattedDocument = serde_json::from_value(document()).unwrap();
        save(&path, good.clone()).unwrap();
        let mut bad = good.clone();
        bad.font.size = 200.;
        assert!(save(&path, bad).unwrap_err().contains("Text size must be between"));
        let mut relative = good.clone();
        relative.blocks = serde_json::from_value(json!([{ "kind": "image", "path": "https://example.com/a.png" }])).unwrap();
        assert!(save(&path, relative).is_err());
        assert_eq!(load(&path).draft.unwrap().document, good);
    }
}
