//! Native Markdown previews with model-provided image loading disabled.
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use pulldown_cmark::{Event, Options, Parser, Tag};
pub(super) fn show(ui: &mut egui::Ui, cache: &mut CommonMarkCache, text: &str) {
    CommonMarkViewer::new().explicit_image_uri_scheme(true).show(ui, cache, &safe_markdown(&super::citations::strip_markers(text)));
}

/// Remove images and non-web hyperlinks before the viewer can ask egui to load or open them.
pub(super) fn safe_markdown(text: &str) -> String {
    let mut parser = Parser::new_ext(text, Options::all()).into_offset_iter();
    let mut out = String::new();
    let mut copied = 0usize;
    while let Some((event, range)) = parser.next() {
        let blocked =
            matches!(&event, Event::Start(Tag::Image { .. })) || matches!(&event, Event::Start(Tag::Link { dest_url, .. }) if !safe_link(dest_url));
        if !blocked {
            continue;
        }
        if let Some(prefix) = text.get(copied..range.start) {
            out.push_str(prefix);
        }
        let mut depth = 1usize;
        let mut end = range.end;
        for (event, range) in parser.by_ref() {
            end = end.max(range.end);
            match event {
                Event::Start(_) => depth = depth.saturating_add(1),
                Event::End(_) => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                Event::Text(t) | Event::Code(t) => {
                    for ch in t.chars() {
                        if "\\`*_{}[]()<>!#$|".contains(ch) {
                            out.push('\\');
                        }
                        out.push(ch);
                    }
                }
                _ => {}
            }
        }
        copied = end;
    }
    if let Some(rest) = text.get(copied..) {
        out.push_str(rest);
    }
    out
}

fn safe_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    ["https://", "http://", "mailto:"].iter().any(|prefix| lower.starts_with(prefix)) && !url.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formatting_survives_while_unsafe_links_and_images_cannot_load() {
        let source = "## Title\n\n| A | B |\n|---|---|\n| one | two |\n\n**Bold** [Source](https://example.com) ![local](file:///private.png) [secret](file:///private.txt) [run](javascript:alert)";
        let safe = safe_markdown(source);
        assert!(safe.contains("| one | two |"));
        assert!(safe.contains("**Bold** [Source](https://example.com)"));
        assert!(!safe.contains("file:"));
        assert!(!safe.contains("javascript:"));
        assert!(!safe.contains("!["));
    }
}
