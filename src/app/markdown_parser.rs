use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// A segment of inline text with styling.
#[derive(Debug, Clone)]
pub(crate) enum TextSegment {
    Plain(String),
    Bold(String),
    Italic(String),
    BoldItalic(String),
    Code(String),
    Link { text: String, url: String },
    Strikethrough(String),
    StrikethroughBold(String),
    StrikethroughItalic(String),
    StrikethroughBoldItalic(String),
    SoftBreak,
    HardBreak,
}

/// A parsed Markdown block ready for rendering.
#[derive(Debug, Clone)]
pub(crate) enum MarkdownBlock {
    Heading {
        level: u8,
        segments: Vec<TextSegment>,
    },
    Paragraph {
        segments: Vec<TextSegment>,
    },
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    List {
        ordered: bool,
        start: Option<u64>,
        items: Vec<Vec<MarkdownBlock>>,
    },
    BlockQuote {
        blocks: Vec<MarkdownBlock>,
    },
    Table {
        headers: Vec<Vec<TextSegment>>,
        rows: Vec<Vec<Vec<TextSegment>>>,
    },
    ThematicBreak,
    Checkbox {
        checked: bool,
        segments: Vec<TextSegment>,
    },
}

/// Parse a Markdown string into a list of `MarkdownBlock`s.
pub(super) fn parse_markdown(input: &str) -> Vec<MarkdownBlock> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(input, opts);
    let events: Vec<Event> = parser.collect();
    parse_blocks(&events)
}

/// Convert a slice of pulldown-cmark events into blocks.
fn parse_blocks(events: &[Event]) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::Heading { level, .. }) => {
                i += 1 + parse_heading(&events[i + 1..], *level, &mut blocks);
            }
            Event::Start(Tag::Paragraph) => {
                i += 1 + parse_paragraph(&events[i + 1..], &mut blocks);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                i += 1 + parse_code_block(&events[i + 1..], kind, &mut blocks);
            }
            Event::Start(Tag::List(start_num)) => {
                i += 1 + parse_list(&events[i + 1..], *start_num, &mut blocks);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                i += 1 + parse_block_quote(&events[i + 1..], &mut blocks);
            }
            Event::Start(Tag::Table(..)) => {
                i += 1 + parse_table(&events[i + 1..], &mut blocks);
            }
            Event::Rule => {
                blocks.push(MarkdownBlock::ThematicBreak);
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    blocks
}

/// Parse a heading start tag, returning the number of events consumed (after the Start tag).
fn parse_heading(
    events: &[Event],
    level: pulldown_cmark::HeadingLevel,
    blocks: &mut Vec<MarkdownBlock>,
) -> usize {
    let (segments, consumed) = collect_inline(events, &TagEnd::Heading(level));
    blocks.push(MarkdownBlock::Heading {
        level: level as u8,
        segments,
    });
    consumed + 1 // +1 for End tag
}

/// Parse a paragraph, handling task list checkboxes. Returns events consumed (after Start tag).
fn parse_paragraph(events: &[Event], blocks: &mut Vec<MarkdownBlock>) -> usize {
    // Check if this paragraph starts with a task list checkbox
    if let Some(Event::TaskListMarker(checked)) = events.first() {
        let checked = *checked;
        let (segs, cons) = collect_inline(&events[1..], &TagEnd::Paragraph);
        blocks.push(MarkdownBlock::Checkbox {
            checked,
            segments: segs,
        });
        1 + cons + 1
    } else {
        let (segments, consumed) = collect_inline(events, &TagEnd::Paragraph);
        blocks.push(MarkdownBlock::Paragraph { segments });
        consumed + 1
    }
}

/// Parse a code block. Returns events consumed (after Start tag).
fn parse_code_block(
    events: &[Event],
    kind: &CodeBlockKind,
    blocks: &mut Vec<MarkdownBlock>,
) -> usize {
    let language = extract_code_language(kind);
    let mut i = 0;
    let mut code = String::new();
    while i < events.len() {
        match &events[i] {
            Event::Text(t) => {
                code.push_str(t);
                i += 1;
            }
            Event::End(TagEnd::CodeBlock) => {
                i += 1;
                break;
            }
            _ => {
                i += 1;
            }
        }
    }
    if code.ends_with('\n') {
        code.pop();
    }
    blocks.push(MarkdownBlock::CodeBlock { language, code });
    i
}

/// Extract language from a code block kind.
fn extract_code_language(kind: &CodeBlockKind) -> Option<String> {
    match kind {
        CodeBlockKind::Fenced(lang) => {
            let l = lang.to_string();
            if l.is_empty() {
                None
            } else {
                Some(l)
            }
        }
        CodeBlockKind::Indented => None,
    }
}

/// Parse a list block. Returns events consumed (after Start tag).
fn parse_list(events: &[Event], start_num: Option<u64>, blocks: &mut Vec<MarkdownBlock>) -> usize {
    let ordered = start_num.is_some();
    let mut i = 0;
    let mut items = Vec::new();
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::Item) => {
                i += 1;
                let (item_blocks, consumed) = collect_item(&events[i..]);
                items.push(item_blocks);
                i += consumed + 1; // +1 for End(Item)
            }
            Event::End(TagEnd::List(_)) => {
                i += 1;
                break;
            }
            _ => {
                i += 1;
            }
        }
    }
    blocks.push(MarkdownBlock::List {
        ordered,
        start: start_num,
        items,
    });
    i
}

/// Parse a block quote. Returns events consumed (after Start tag).
fn parse_block_quote(events: &[Event], blocks: &mut Vec<MarkdownBlock>) -> usize {
    let mut i = 0;
    let mut depth = 1;
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::BlockQuote(_)) => {
                depth += 1;
                i += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }
    let inner_blocks = parse_blocks(&events[..i]);
    blocks.push(MarkdownBlock::BlockQuote {
        blocks: inner_blocks,
    });
    i + 1 // +1 for End(BlockQuote)
}

/// Parse a table block. Returns events consumed (after Start tag).
fn parse_table(events: &[Event], blocks: &mut Vec<MarkdownBlock>) -> usize {
    let mut i = 0;
    let mut headers = Vec::new();
    let mut rows = Vec::new();

    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::TableHead) => {
                i += 1;
                i += collect_table_cells(&events[i..], &TagEnd::TableHead, &mut headers);
            }
            Event::Start(Tag::TableRow) => {
                i += 1;
                let mut row = Vec::new();
                i += collect_table_cells(&events[i..], &TagEnd::TableRow, &mut row);
                rows.push(row);
            }
            Event::End(TagEnd::Table) => {
                i += 1;
                break;
            }
            _ => {
                i += 1;
            }
        }
    }
    blocks.push(MarkdownBlock::Table { headers, rows });
    i
}

/// Collect table cells until the given end tag. Returns events consumed.
fn collect_table_cells(
    events: &[Event],
    end_tag: &TagEnd,
    cells: &mut Vec<Vec<TextSegment>>,
) -> usize {
    let mut i = 0;
    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::TableCell) => {
                i += 1;
                let (segs, consumed) = collect_inline(&events[i..], &TagEnd::TableCell);
                cells.push(segs);
                i += consumed + 1;
            }
            Event::End(tag) if tag == end_tag => {
                i += 1;
                break;
            }
            _ => {
                i += 1;
            }
        }
    }
    i
}

/// Collect list item content, which may contain paragraphs, sub-lists, etc.
/// Handles both "tight" items (bare inline content) and "loose" items (wrapped in Paragraph).
fn collect_item(events: &[Event]) -> (Vec<MarkdownBlock>, usize) {
    let mut i = 0;
    let mut item_events = Vec::new();
    let mut depth = 0;

    while i < events.len() {
        match &events[i] {
            Event::Start(Tag::Item) => {
                depth += 1;
                item_events.push(events[i].clone());
                i += 1;
            }
            Event::End(TagEnd::Item) => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                item_events.push(events[i].clone());
                i += 1;
            }
            _ => {
                item_events.push(events[i].clone());
                i += 1;
            }
        }
    }

    // Detect tight vs loose list items.
    // Tight items have bare inline content (no Paragraph/block-level wrapper).
    let is_block_content = item_events.first().is_none_or(|ev| {
        matches!(
            ev,
            Event::Start(Tag::Paragraph)
                | Event::Start(Tag::CodeBlock(_))
                | Event::Start(Tag::List(_))
                | Event::Start(Tag::BlockQuote(_))
                | Event::Start(Tag::Table(_))
                | Event::Start(Tag::Heading { .. })
        )
    });

    if is_block_content {
        let blocks = parse_blocks(&item_events);
        (blocks, i)
    } else {
        // Tight list item: collect inline segments directly.
        // Check for task list marker first.
        let (skip, checkbox) = if let Some(Event::TaskListMarker(checked)) = item_events.first() {
            (1, Some(*checked))
        } else {
            (0, None)
        };
        // Use TagEnd::Item as sentinel — it won't appear in item_events since we
        // break before adding End(Item) at depth 0.
        let (segments, _) = collect_inline(&item_events[skip..], &TagEnd::Item);
        let block = if let Some(checked) = checkbox {
            MarkdownBlock::Checkbox { checked, segments }
        } else {
            MarkdownBlock::Paragraph { segments }
        };
        (vec![block], i)
    }
}

/// Map text to the appropriate styled segment based on current formatting state.
fn styled_text(text: String, bold: bool, italic: bool, strikethrough: bool) -> TextSegment {
    match (strikethrough, bold, italic) {
        (true, true, true) => TextSegment::StrikethroughBoldItalic(text),
        (true, true, false) => TextSegment::StrikethroughBold(text),
        (true, false, true) => TextSegment::StrikethroughItalic(text),
        (true, false, false) => TextSegment::Strikethrough(text),
        (false, true, true) => TextSegment::BoldItalic(text),
        (false, true, false) => TextSegment::Bold(text),
        (false, false, true) => TextSegment::Italic(text),
        (false, false, false) => TextSegment::Plain(text),
    }
}

/// Collect link text events until End(Link). Returns (link_text, events_consumed).
fn collect_link_text(events: &[Event]) -> (String, usize) {
    let mut link_text = String::new();
    let mut i = 0;
    while i < events.len() {
        match &events[i] {
            Event::Text(t) => {
                link_text.push_str(t);
                i += 1;
            }
            Event::Code(c) => {
                link_text.push_str(c);
                i += 1;
            }
            Event::End(TagEnd::Link) => {
                i += 1;
                break;
            }
            _ => {
                i += 1;
            }
        }
    }
    (link_text, i)
}

/// Collect inline events (text, bold, italic, code, links) until the matching End tag.
fn collect_inline(events: &[Event], end_tag: &TagEnd) -> (Vec<TextSegment>, usize) {
    let mut segments = Vec::new();
    let mut i = 0;
    let mut bold = false;
    let mut italic = false;
    let mut strikethrough = false;

    while i < events.len() {
        match &events[i] {
            Event::End(tag) if tag == end_tag => {
                return (segments, i);
            }
            Event::Text(text) => {
                segments.push(styled_text(text.to_string(), bold, italic, strikethrough));
                i += 1;
            }
            Event::Code(code) => {
                segments.push(TextSegment::Code(code.to_string()));
                i += 1;
            }
            Event::SoftBreak => {
                segments.push(TextSegment::SoftBreak);
                i += 1;
            }
            Event::HardBreak => {
                segments.push(TextSegment::HardBreak);
                i += 1;
            }
            Event::Start(Tag::Strong) => {
                bold = true;
                i += 1;
            }
            Event::End(TagEnd::Strong) => {
                bold = false;
                i += 1;
            }
            Event::Start(Tag::Emphasis) => {
                italic = true;
                i += 1;
            }
            Event::End(TagEnd::Emphasis) => {
                italic = false;
                i += 1;
            }
            Event::Start(Tag::Strikethrough) => {
                strikethrough = true;
                i += 1;
            }
            Event::End(TagEnd::Strikethrough) => {
                strikethrough = false;
                i += 1;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let url = dest_url.to_string();
                let (link_text, consumed) = collect_link_text(&events[i + 1..]);
                segments.push(TextSegment::Link {
                    text: link_text,
                    url,
                });
                i += 1 + consumed;
            }
            Event::TaskListMarker(checked) => {
                let prefix = if *checked { "[x] " } else { "[ ] " };
                segments.push(TextSegment::Plain(prefix.to_string()));
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    (segments, i)
}
