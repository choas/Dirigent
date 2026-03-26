/// Check whether a trimmed line is HTML markup that should be skipped entirely.
/// These are lines that are purely structural/decorative and contain no useful text.
pub(super) fn is_skippable_markup(trimmed: &str) -> bool {
    trimmed.starts_with("<!--")
        || trimmed.starts_with("<sub")
        || trimmed.starts_with("</sub")
        || trimmed.starts_with("<blockquote")
        || trimmed.starts_with("</blockquote")
        || trimmed.starts_with("![")
        // Full-line image tags (logos, dividers, badges)
        || (trimmed.starts_with("<img ") && !trimmed.to_ascii_lowercase().contains("alt=\"action required\""))
        || trimmed.starts_with("<br")
        || trimmed == "<br/>"
        || trimmed == "<br />"
        // Lines that are purely an HTML link wrapping an image (e.g. Qodo logo)
        || (trimmed.starts_with("<a ") && trimmed.contains("<img ") && trimmed.ends_with("</a>"))
}

/// Strip inline HTML tags from a string, preserving the text content.
/// Converts `<b>`, `<i>`, `<code>`, `<pre>` etc. to their text content,
/// drops self-closing tags like `<img .../>` and `<br/>`.
/// Also decodes HTML entities (`&amp;`, `&lt;`, `&#123;`, `&#x1F600;`, etc.).
fn is_known_html_tag(name: &str) -> bool {
    matches!(
        name,
        "a" | "abbr"
            | "b"
            | "blockquote"
            | "br"
            | "caption"
            | "cite"
            | "code"
            | "col"
            | "colgroup"
            | "dd"
            | "del"
            | "details"
            | "dfn"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "figcaption"
            | "figure"
            | "font"
            | "footer"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "i"
            | "img"
            | "ins"
            | "kbd"
            | "li"
            | "mark"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "q"
            | "s"
            | "samp"
            | "section"
            | "small"
            | "span"
            | "strike"
            | "strong"
            | "sub"
            | "summary"
            | "sup"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
            | "tt"
            | "u"
            | "ul"
            | "var"
    )
}

pub(crate) fn strip_html_tags(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            consume_tag(&mut chars, &mut result);
        } else if ch == '&' {
            consume_entity(&mut chars, &mut result);
        } else {
            result.push(ch);
        }
    }
    result
}

/// Consume an HTML tag from `chars` (the `<` has already been consumed).
/// Known tags are stripped; unknown angle-bracketed text is preserved verbatim.
fn consume_tag(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, result: &mut String) {
    let mut tag = String::new();
    let mut found_close = false;
    for inner in chars.by_ref() {
        if inner == '>' {
            found_close = true;
            break;
        }
        tag.push(inner);
    }

    if !found_close {
        result.push('<');
        result.push_str(&tag);
        return;
    }

    let tag_trimmed = tag.trim_start_matches('/');
    let tag_name: String = tag_trimmed
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '/')
        .collect();
    let tag_name_lower = tag_name.to_lowercase();

    if !is_known_html_tag(&tag_name_lower) {
        result.push('<');
        result.push_str(&tag);
        if found_close {
            result.push('>');
        }
        return;
    }

    if tag_name_lower == "br"
        && !result.is_empty()
        && !result.ends_with(' ')
        && !result.ends_with('\n')
    {
        result.push(' ');
    }
}

/// Consume an HTML entity from `chars` (the `&` has already been consumed).
/// Decoded entities are appended; unknown or malformed entities are preserved as-is.
fn consume_entity(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, result: &mut String) {
    let mut entity = String::new();
    let mut found_semicolon = false;
    while let Some(&next) = chars.peek() {
        if next == ';' {
            chars.next();
            found_semicolon = true;
            break;
        }
        if entity.len() > 10 || next.is_whitespace() || next == '<' || next == '&' {
            break;
        }
        entity.push(next);
        chars.next();
    }
    if found_semicolon {
        match decode_html_entity(&entity) {
            Some(decoded) => result.push(decoded),
            None => {
                result.push('&');
                result.push_str(&entity);
                result.push(';');
            }
        }
    } else {
        result.push('&');
        result.push_str(&entity);
    }
}

/// Decode a single HTML entity name (without the `&` and `;`).
/// Handles common named entities and numeric entities (`#123`, `#x1F600`).
fn decode_html_entity(entity: &str) -> Option<char> {
    // Numeric entities
    if let Some(rest) = entity.strip_prefix('#') {
        let code = if let Some(hex) = rest.strip_prefix('x').or_else(|| rest.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            rest.parse::<u32>().ok()?
        };
        return char::from_u32(code);
    }
    // Named entities
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{00A0}'),
        "ndash" => Some('\u{2013}'),
        "mdash" => Some('\u{2014}'),
        "lsquo" => Some('\u{2018}'),
        "rsquo" => Some('\u{2019}'),
        "ldquo" => Some('\u{201C}'),
        "rdquo" => Some('\u{201D}'),
        "bull" => Some('\u{2022}'),
        "hellip" => Some('\u{2026}'),
        "copy" => Some('\u{00A9}'),
        "reg" => Some('\u{00AE}'),
        "trade" => Some('\u{2122}'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_tags_preserves_text() {
        assert_eq!(strip_html_tags("hello <b>world</b>"), "hello world");
        assert_eq!(strip_html_tags("<code>foo</code>"), "foo");
        assert_eq!(strip_html_tags("a<br/>b"), "a b");
        assert_eq!(strip_html_tags("no tags"), "no tags");
        assert_eq!(strip_html_tags("<h3>Title</h3>"), "Title");
        assert_eq!(
            strip_html_tags(r#"<a href="url">link text</a>"#),
            "link text"
        );
    }

    #[test]
    fn strip_html_tags_preserves_non_html_angle_brackets() {
        // Generic type parameters should be preserved
        assert_eq!(strip_html_tags("Vec<T>"), "Vec<T>");
        // JSX-style components should be preserved
        assert_eq!(strip_html_tags("<Button />"), "<Button />");
        assert_eq!(
            strip_html_tags("<MyComponent>child</MyComponent>"),
            "<MyComponent>child</MyComponent>"
        );
        // Mixed: known HTML tags stripped, unknown preserved
        assert_eq!(strip_html_tags("<code>Vec<T></code>"), "Vec<T>");
        assert_eq!(
            strip_html_tags("use <b>HashMap</b><K, V>"),
            "use HashMap<K, V>"
        );
    }

    #[test]
    fn strip_html_tags_decodes_entities() {
        assert_eq!(strip_html_tags("Hello &amp; World"), "Hello & World");
        assert_eq!(strip_html_tags("&lt;code&gt;"), "<code>");
        assert_eq!(strip_html_tags("a &amp; b &amp; c"), "a & b & c");
        assert_eq!(strip_html_tags("&quot;quoted&quot;"), "\"quoted\"");
        assert_eq!(strip_html_tags("&#60;tag&#62;"), "<tag>");
        assert_eq!(strip_html_tags("&#x3C;hex&#x3E;"), "<hex>");
        assert_eq!(strip_html_tags("no&amp;space"), "no&space");
        assert_eq!(strip_html_tags("<b>&amp;</b> &lt;ok&gt;"), "& <ok>");
        // Unknown entity preserved as-is
        assert_eq!(strip_html_tags("&unknown;"), "&unknown;");
        // Bare ampersand (no semicolon) preserved
        assert_eq!(strip_html_tags("a & b"), "a & b");
    }
}
