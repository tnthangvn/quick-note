//! Markdown export of pages (human-readable copy on Drive / disk).

use crate::model::Page;

pub fn page_to_markdown(page: &Page) -> String {
    let mut out = format!("# {}\n", page.title);
    for note in &page.notes {
        out.push_str(&format!("\n## {}\n\n", note.display_title()));
        let body = note.body.trim_end();
        if !body.is_empty() {
            out.push_str(body);
            out.push('\n');
        }
    }
    if !page.links.is_empty() {
        out.push_str("\n## Liên kết\n\n");
        let title = |id| {
            page.notes
                .iter()
                .find(|n| n.id == id)
                .map(|n| n.display_title())
                .unwrap_or("?")
        };
        for link in &page.links {
            out.push_str(&format!("- {} → {}", title(link.from), title(link.to)));
            if !link.label.trim().is_empty() {
                out.push_str(&format!(" ({})", link.label.trim()));
            }
            out.push('\n');
        }
    }
    out
}

/// File name safe on Drive and every desktop OS.
pub fn safe_file_name(title: &str, ext: &str) -> String {
    let cleaned: String = title
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let base = if cleaned.is_empty() {
        "untitled".to_string()
    } else {
        cleaned
    };
    format!("{base}.{ext}")
}

/// One `.md` name per page; duplicates get " (2)", " (3)"… so no page overwrites another.
pub fn unique_file_names(pages: &[Page]) -> Vec<String> {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    pages
        .iter()
        .map(|page| {
            let base = safe_file_name(&page.title, "md");
            let count = seen.entry(base.to_lowercase()).or_insert(0);
            *count += 1;
            match *count {
                1 => base,
                n => format!("{} ({n}).md", base.trim_end_matches(".md")),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_has_page_and_note_headings() {
        let mut page = Page::new("Công việc");
        page.add_note([0.0, 0.0]);
        page.notes[0].title = "Hôm nay".into();
        page.notes[0].body = "- viết code\n".into();
        let md = page_to_markdown(&page);
        assert_eq!(md, "# Công việc\n\n## Hôm nay\n\n- viết code\n");
    }

    #[test]
    fn duplicate_titles_get_distinct_names() {
        let pages = [
            Page::new("Việc"),
            Page::new("việc"),
            Page::new(" "),
            Page::new(""),
        ];
        assert_eq!(
            unique_file_names(&pages),
            ["Việc.md", "việc (2).md", "untitled.md", "untitled (2).md"]
        );
    }

    #[test]
    fn markdown_lists_links() {
        let mut page = Page::new("P");
        let a = page.add_note([0.0, 0.0]);
        let b = page.add_note([0.0, 0.0]);
        page.notes[0].title = "A".into();
        page.notes[1].title = "B".into();
        page.connect(a, b);
        page.links[0].label = "gọi".into();
        assert!(page_to_markdown(&page).ends_with("## Liên kết\n\n- A → B (gọi)\n"));
    }

    #[test]
    fn file_name_strips_unsafe_chars() {
        assert_eq!(safe_file_name("a/b:c?", "md"), "a_b_c_.md");
        assert_eq!(safe_file_name("   ", "md"), "untitled.md");
    }
}
