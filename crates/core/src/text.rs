use crate::{Entry, Store};
use anyhow::Result;
use std::time::Duration;

pub const TEXT_LIMIT: usize = 512 * 1024;

pub fn file_text(store: &Store, entry: Option<Entry>) -> Result<Option<String>> {
    let Some(entry) = entry else {
        return Ok(Some(String::new()));
    };
    if store.manifest(entry.id)?.size > TEXT_LIMIT as u64 {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    store.copy_file(entry.id, &mut bytes)?;
    Ok(String::from_utf8(bytes).ok().filter(|s| {
        s.chars()
            .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
    }))
}

pub fn unified_patch(path: &str, before: &str, after: &str) -> String {
    similar::TextDiff::configure()
        .timeout(Duration::from_millis(150))
        .diff_lines(before, after)
        .unified_diff()
        .header(path, path)
        .to_string()
}
