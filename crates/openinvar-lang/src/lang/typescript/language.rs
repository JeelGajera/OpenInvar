use std::path::Path;

use openinvar_core::ir::Language;

pub fn detect_language(path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte" | "astro" => Some(Language::TypeScript),
        "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
        _ => None,
    }
}

pub fn is_supported_source_file(path: &Path) -> bool {
    detect_language(path).is_some()
}
