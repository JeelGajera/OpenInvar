//! Framework pre-processing for Vue/Svelte/Astro files.
//!
//! The adapter extracts script-bearing regions and replaces everything else with
//! whitespace while preserving newline positions. This keeps parser diagnostics
//! and relationship line numbers aligned with original source files.

use crate::lang::typescript::parser::FrameworkKind;

/// Extract script-capable content from framework source.
///
/// Non-script bytes are replaced with spaces while preserving newline positions.
pub fn extract_script_content(source: &str, kind: FrameworkKind) -> String {
    match kind {
        FrameworkKind::Vue => extract_vue_script(source),
        FrameworkKind::Svelte => extract_svelte_script(source),
        FrameworkKind::Astro => extract_astro_frontmatter(source),
        FrameworkKind::None => source.to_string(),
    }
}

/// Extract all Vue `<script ...>...</script>` regions.
fn extract_vue_script(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut result = blank_preserving_newlines(bytes);

    let mut search_start = 0usize;
    while search_start < source.len() {
        let Some(tag_start) =
            find_pattern_from(&source[search_start..], "<script").map(|p| p + search_start)
        else {
            break;
        };

        let Some(tag_end_rel) = find_pattern_from(&source[tag_start..], ">") else {
            break;
        };
        let content_start = tag_start + tag_end_rel + 1;

        let Some(close_rel) = find_pattern_from(&source[content_start..], "</script>") else {
            break;
        };
        let content_end = content_start + close_rel;

        copy_region(bytes, &mut result, content_start, content_end);
        search_start = content_end + "</script>".len();
    }

    String::from_utf8(result).unwrap_or_else(|_| " ".repeat(source.len()))
}

/// Extract all Svelte `<script ...>...</script>` regions.
fn extract_svelte_script(source: &str) -> String {
    extract_vue_script(source)
}

/// Extract Astro frontmatter (`--- ... ---`) content.
fn extract_astro_frontmatter(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut result = blank_preserving_newlines(bytes);

    if !source.starts_with("---") {
        return String::from_utf8(result).unwrap_or_else(|_| " ".repeat(source.len()));
    }

    let mut search_start = 3usize;
    if source.as_bytes().get(search_start) == Some(&b'\r')
        && source.as_bytes().get(search_start + 1) == Some(&b'\n')
    {
        search_start += 2;
    } else if source.as_bytes().get(search_start) == Some(&b'\n') {
        search_start += 1;
    }

    let Some(close_rel) = find_pattern_from(&source[search_start..], "\n---") else {
        copy_region(bytes, &mut result, 0, source.len());
        return String::from_utf8(result).unwrap_or_else(|_| " ".repeat(source.len()));
    };

    let frontmatter_end = search_start + close_rel;
    copy_region(bytes, &mut result, search_start, frontmatter_end);

    String::from_utf8(result).unwrap_or_else(|_| " ".repeat(source.len()))
}

fn blank_preserving_newlines(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .map(|&b| if b == b'\n' { b'\n' } else { b' ' })
        .collect()
}

fn copy_region(src: &[u8], dst: &mut [u8], start: usize, end: usize) {
    let bounded_end = end.min(src.len()).min(dst.len());
    let bounded_start = start.min(bounded_end);
    dst[bounded_start..bounded_end].copy_from_slice(&src[bounded_start..bounded_end]);
}

fn find_pattern_from(haystack: &str, pattern: &str) -> Option<usize> {
    haystack.find(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vue_script_extraction_preserves_line_numbers() {
        let source = "<template>\n  <div>hello</div>\n</template>\n<script setup lang=\"ts\">\nimport { ref } from 'vue';\nconst count = ref(0);\n</script>\n";
        let extracted = extract_vue_script(source);
        let lines: Vec<&str> = extracted.lines().collect();
        assert!(
            lines[3].trim().is_empty(),
            "script tag line should be blanked"
        );
        assert!(lines[4].contains("import"), "line 5 should contain import");
        assert!(lines[5].contains("count"), "line 6 should contain count");
        assert_eq!(
            extracted.lines().count(),
            source.lines().count(),
            "line count should be preserved"
        );
    }

    #[test]
    fn test_astro_frontmatter_extraction_preserves_line_numbers() {
        let source = "---\nimport { Component } from './component';\nconst title = 'hello';\n---\n<html>\n  <body>{title}</body>\n</html>\n";
        let extracted = extract_astro_frontmatter(source);
        let lines: Vec<&str> = extracted.lines().collect();
        assert!(lines[1].contains("import"), "line 2 should contain import");
        assert!(lines[2].contains("title"), "line 3 should contain title");
        assert!(
            lines[4].trim().is_empty(),
            "template line should be blanked"
        );
        assert_eq!(
            extracted.lines().count(),
            source.lines().count(),
            "line count should be preserved"
        );
    }

    #[test]
    fn test_svelte_script_extraction() {
        let source =
            "<script lang=\"ts\">\nexport let name: string;\n</script>\n\n<h1>{name}</h1>\n";
        let extracted = extract_svelte_script(source);
        let lines: Vec<&str> = extracted.lines().collect();
        assert!(
            lines[1].contains("export"),
            "script body should be preserved"
        );
        assert!(
            lines[3].trim().is_empty(),
            "template lines should be blanked"
        );
        assert!(
            lines[4].trim().is_empty(),
            "template lines should be blanked"
        );
    }

    #[test]
    fn test_blank_preserving_newlines_preserves_all_newlines() {
        let source = "hello\nworld\nfoo\n";
        let blanked = blank_preserving_newlines(source.as_bytes());
        let result = String::from_utf8(blanked).expect("utf8");
        assert_eq!(
            result.lines().count(),
            source.lines().count(),
            "line count should be preserved"
        );
        for (a, b) in source.chars().zip(result.chars()) {
            if a == '\n' {
                assert_eq!(b, '\n', "newline should be preserved");
            } else {
                assert_eq!(b, ' ', "non-newline should be replaced with space");
            }
        }
    }

    #[test]
    fn test_vue_with_no_script_block_returns_blanked() {
        let source = "<template>\n  <div>just template</div>\n</template>\n";
        let extracted = extract_vue_script(source);
        for c in extracted.chars() {
            assert!(
                c == ' ' || c == '\n',
                "only spaces/newlines expected in blanked output"
            );
        }
    }
}
