//! Writer patches to the text element (see `docs/patches/`): blocks below lines and overlays on
//! parts of lines, and placing a line's rows by its [`crate::input::LineStyle`] (first-line indent,
//! alignment, justification).

use super::*;
use crate::input::display_map::{InlineFragment, InputLine};

impl<M: InputModeKind> TextElement<M> {
    /// Lay out the blocks below the visible lines, each at the bottom of its line.
    pub(super) fn prepaint_line_blocks(
        &self,
        layout: &LastLayout,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let state = self.state.read(cx);
        let Some(render) = state.line_block_renderer.clone() else {
            return vec![];
        };
        let width = layout.wrap_width.unwrap_or(layout.content_width);
        let mut placements = Vec::new();
        // The header above the first line, while that line is laid out.
        let inset = state.display_map.top_inset();
        if inset > px(0.) && layout.visible_buffer_lines.first() == Some(&0) {
            placements.push((
                crate::input::LineBlockContext {
                    line: 0,
                    text: state.text.slice_line(0).to_string().into(),
                    size: size(width, inset),
                    above: true,
                },
                bounds.origin + point(layout.line_number_width, layout.visible_top - inset),
            ));
        }
        let mut y = layout.visible_top;
        for (line, &row) in layout.lines.iter().zip(layout.visible_buffer_lines.iter()) {
            let line_size = line.size(layout.line_height);
            // The space after the block (see `LineStyle::space_after`) has nothing to draw.
            let content_height = line.block_height - line.block_space;
            if content_height > px(0.) {
                let top = y + line_size.height - line.block_height;
                placements.push((
                    crate::input::LineBlockContext {
                        line: row,
                        text: state.text.slice_line(row).to_string().into(),
                        size: size(width, content_height),
                        above: false,
                    },
                    bounds.origin + point(layout.line_number_width, top),
                ));
            }
            y += line_size.height;
        }
        placements
            .into_iter()
            .map(|(context, origin)| {
                let mut element = render(&context, window, cx);
                element.layout_as_root(
                    context.size.map(gpui::AvailableSpace::Definite),
                    window,
                    cx,
                );
                element.prepaint_at(origin, window, cx);
                element
            })
            .collect()
    }

    /// Lay out the overlays on parts of the visible lines (see
    /// [`crate::input::EditorState::set_line_overlays`]). Lines a selection touches get none, so
    /// their text shows for editing.
    pub(super) fn prepaint_line_overlays(
        &self,
        layout: &LastLayout,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        let state = self.state.read(cx);
        let Some((ranges, render)) = state.line_overlays.clone() else {
            return vec![];
        };
        let selected_rows: Vec<Range<usize>> = state
            .selections
            .iter()
            .map(|selection| {
                let start = state
                    .text
                    .offset_to_point(selection.start.min(state.text.len()));
                let end = state
                    .text
                    .offset_to_point(selection.end.min(state.text.len()));
                start.row..end.row + 1
            })
            .collect();
        let mut placements = Vec::new();
        let mut y = layout.visible_top;
        for (line, &row) in layout.lines.iter().zip(layout.visible_buffer_lines.iter()) {
            let line_size = line.size(layout.line_height);
            // A collapsed line (see `LineStyle::collapsed`) has no text rows to cover.
            let has_text = !line.is_hidden();
            if has_text && !selected_rows.iter().any(|rows| rows.contains(&row)) {
                let text: SharedString = state.text.slice_line(row).to_string().into();
                let line_start = state.text.line_start_offset(row);
                let spans = state.token_spans();
                for range in ranges(row, &text) {
                    if range.is_empty() || range.end > text.len() {
                        continue;
                    }
                    // An inline object already replaces that text (`set_inline_objects`).
                    let absolute = line_start + range.start..line_start + range.end;
                    if spans.iter().any(|span| {
                        let span = span.range();
                        span.start < absolute.end && absolute.start < span.end
                    }) {
                        continue;
                    }
                    let (Some(start), Some(end)) = (
                        line.position_for_index(range.start, layout, false),
                        line.position_for_index(range.end, layout, true),
                    ) else {
                        continue;
                    };
                    if start.y != end.y || end.x <= start.x {
                        continue;
                    }
                    placements.push((
                        crate::input::LineOverlayContext {
                            line: row,
                            text: text.clone(),
                            range,
                            size: size(end.x - start.x, line.row_height_or(layout.line_height)),
                        },
                        bounds.origin + start + point(layout.line_number_width, y),
                    ));
                }
            }
            y += line_size.height;
        }
        placements
            .into_iter()
            .map(|(context, origin)| {
                let mut element = render(&context, window, cx);
                element.layout_as_root(
                    context.size.map(gpui::AvailableSpace::Definite),
                    window,
                    cx,
                );
                element.prepaint_at(origin, window, cx);
                element
            })
            .collect()
    }
}
/// Visual rows of a line, placed by its [`crate::input::LineStyle`].
pub(super) enum Placed {
    /// No first-line indent or alignment: rows as shaped.
    Plain(SmallVec<[ShapedLine; 1]>),
    /// Rows (justified ones split into spaced fragments) and each row's left offset.
    Rows(SmallVec<[InputLine; 1]>, SmallVec<[Pixels; 1]>),
}

/// Place a line's shaped rows: first-line indent, alignment in the wrap width, and justification
/// of every row but the last (see [`crate::input::LineAlign`]).
pub(super) fn place_rows(
    rows: SmallVec<[ShapedLine; 1]>,
    runs: &[Vec<TextRun>],
    style: &crate::input::LineStyle,
    wrap_indent: Pixels,
    wrap_width: Option<Pixels>,
    font_size: Pixels,
    window: &mut Window,
) -> Placed {
    use crate::input::LineAlign;
    if style.first_indent <= px(0.) && style.align == LineAlign::Start {
        return Placed::Plain(rows);
    }
    let last = rows.len().saturating_sub(1);
    let mut placed = SmallVec::with_capacity(rows.len());
    let mut offsets = SmallVec::with_capacity(rows.len());
    for (ix, (row, row_runs)) in rows.into_iter().zip(runs).enumerate() {
        let indent = if ix == 0 {
            style.first_indent
        } else {
            wrap_indent
        };
        // Trailing spaces at a wrap point do not count toward the row's width.
        let width = row.x_for_index(row.text.trim_end().len());
        let free = wrap_width.map_or(px(0.), |wrap| (wrap - indent - width).max(px(0.)));
        let offset = match style.align {
            LineAlign::Center => indent + free / 2.,
            LineAlign::End => indent + free,
            LineAlign::Start | LineAlign::Justify => indent,
        };
        offsets.push(offset);
        let justified = (style.align == LineAlign::Justify && ix < last && free > px(0.))
            .then(|| justify_row(&row, row_runs, free, font_size, window))
            .flatten();
        placed.push(justified.unwrap_or_else(|| InputLine::from(row)));
    }
    Placed::Rows(placed, offsets)
}

/// A visual row that holds inline objects (see `EditorState::set_inline_objects`): the text
/// between objects shaped with the row's runs and font size, and each object an unpainted gap as
/// wide as its measured element (the element is painted over it). `row_start` is the row's offset
/// in the document. `None` when no object lies wholly in the row.
pub(super) fn object_row(
    text: &SharedString,
    row_start: usize,
    spans: &[crate::input::InlineTokenSpan],
    cache: &super::super::token_presentation::TokenLayoutCache,
    runs: &[TextRun],
    font_size: Pixels,
    window: &mut Window,
) -> Option<InputLine> {
    let row_end = row_start + text.len();
    let first = spans.partition_point(|span| span.range().end <= row_start);
    let objects: Vec<(Range<usize>, Pixels)> = spans[first..]
        .iter()
        .take_while(|span| span.range().start < row_end)
        .filter(|span| span.range().start >= row_start && span.range().end <= row_end)
        .map(|span| {
            let range = span.range();
            let width = cache.widths.get(span.token()).copied().unwrap_or_default();
            (range.start - row_start..range.end - row_start, width)
        })
        .collect();
    if objects.is_empty() {
        return None;
    }
    let shape = |range: Range<usize>, x: Pixels, window: &mut Window| {
        let piece: SharedString = text[range.clone()].to_string().into();
        let piece_runs = runs_for_range(runs, 0, &range);
        let piece_runs = align_runs_to_char_boundaries(&piece, &piece_runs).unwrap_or(piece_runs);
        let shaped = window
            .text_system()
            .shape_line(piece, font_size, &piece_runs, None);
        InlineFragment {
            x,
            width: shaped.width,
            text: Some(shaped),
            range,
        }
    };
    let mut fragments = Vec::with_capacity(objects.len() * 2 + 1);
    let (mut offset, mut x) = (0, px(0.));
    for (range, width) in objects {
        if offset < range.start {
            let fragment = shape(offset..range.start, x, window);
            x += fragment.width;
            fragments.push(fragment);
        }
        fragments.push(InlineFragment {
            range: range.clone(),
            x,
            width,
            text: None,
        });
        x += width;
        offset = range.end;
    }
    if offset < text.len() {
        fragments.push(shape(offset..text.len(), x, window));
    }
    Some(InputLine::inline(text.clone(), fragments))
}

/// [`place_rows`] for a line with object rows (see [`object_row`]): `object_rows` replace the
/// shaped rows at their indexes. Object rows are indented and aligned like the others but not
/// justified (an object cannot stretch).
#[allow(clippy::too_many_arguments)]
pub(super) fn place_object_rows(
    rows: SmallVec<[ShapedLine; 1]>,
    object_rows: Vec<(usize, InputLine)>,
    runs: &[Vec<TextRun>],
    style: &crate::input::LineStyle,
    wrap_indent: Pixels,
    wrap_width: Option<Pixels>,
    font_size: Pixels,
    window: &mut Window,
) -> Placed {
    use crate::input::LineAlign;
    let last = rows.len().saturating_sub(1);
    let mut object_rows = object_rows.into_iter().peekable();
    let mut placed = SmallVec::with_capacity(rows.len());
    let mut offsets = SmallVec::with_capacity(rows.len());
    for (ix, (row, row_runs)) in rows.into_iter().zip(runs).enumerate() {
        let object = object_rows
            .next_if(|(at, _)| *at == ix)
            .map(|(_, line)| line);
        let indent = if ix == 0 {
            style.first_indent
        } else {
            wrap_indent
        };
        let text_len = row.text.trim_end().len();
        let width = match &object {
            Some(line) => line.x_for_index(text_len),
            None => row.x_for_index(text_len),
        };
        let free = wrap_width.map_or(px(0.), |wrap| (wrap - indent - width).max(px(0.)));
        let offset = match style.align {
            LineAlign::Center => indent + free / 2.,
            LineAlign::End => indent + free,
            LineAlign::Start | LineAlign::Justify => indent,
        };
        offsets.push(offset);
        let line = match object {
            Some(line) => line,
            None => (style.align == LineAlign::Justify && ix < last && free > px(0.))
                .then(|| justify_row(&row, row_runs, free, font_size, window))
                .flatten()
                .unwrap_or_else(|| InputLine::from(row)),
        };
        placed.push(line);
    }
    Placed::Rows(placed, offsets)
}

/// Stretch a row by `free`, spread over its runs of spaces, or between its graphemes when it has
/// no spaces (e.g. CJK text). Words are shaped on their own and painted at their widened
/// positions; a run of spaces becomes an unpainted gap as wide as the spaces plus its share. So
/// caret, selection and hit testing follow the stretched row.
fn justify_row(
    row: &ShapedLine,
    runs: &[TextRun],
    free: Pixels,
    font_size: Pixels,
    window: &mut Window,
) -> Option<InputLine> {
    use unicode_segmentation::UnicodeSegmentation as _;
    let text = row.text.as_ref();
    let content_len = text.trim_end().len();
    let content = &text[..content_len];
    // (word, spaces after it) byte ranges. Trailing spaces stay with the last word.
    let mut pieces: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    let mut word_start = 0;
    let mut chars = content.char_indices().peekable();
    while let Some((ix, c)) = chars.next() {
        if c.is_whitespace() && ix > word_start {
            let mut end = ix + c.len_utf8();
            while let Some(&(next, c)) = chars.peek() {
                if !c.is_whitespace() {
                    break;
                }
                end = next + c.len_utf8();
                chars.next();
            }
            if end < content_len {
                pieces.push((word_start..ix, ix..end));
                word_start = end;
            }
        }
    }
    if pieces.is_empty() {
        // No spaces: widen between graphemes instead.
        let ends: Vec<usize> = content
            .grapheme_indices(true)
            .skip(1)
            .map(|(ix, _)| ix)
            .collect();
        let mut start = 0;
        for end in ends {
            pieces.push((start..end, end..end));
            start = end;
        }
        word_start = start;
    }
    if pieces.is_empty() {
        return None;
    }
    let extra = free / pieces.len() as f32;
    let shape = |range: Range<usize>, window: &mut Window| {
        let piece: SharedString = text[range.clone()].to_string().into();
        window
            .text_system()
            .shape_line(piece, font_size, &runs_for_range(runs, 0, &range), None)
    };
    let mut fragments = Vec::with_capacity(pieces.len() * 2 + 1);
    for (gap, (word, spaces)) in pieces.into_iter().enumerate() {
        let shift = extra * gap as f32;
        fragments.push(InlineFragment {
            x: row.x_for_index(word.start) + shift,
            width: row.x_for_index(word.end) - row.x_for_index(word.start),
            text: Some(shape(word.clone(), window)),
            range: word,
        });
        fragments.push(InlineFragment {
            x: row.x_for_index(spaces.start) + shift,
            width: row.x_for_index(spaces.end) - row.x_for_index(spaces.start) + extra,
            text: None,
            range: spaces,
        });
    }
    let last = word_start..text.len();
    fragments.push(InlineFragment {
        x: row.x_for_index(last.start) + free,
        width: row.x_for_index(last.end) - row.x_for_index(last.start),
        text: Some(shape(last.clone(), window)),
        range: last,
    });
    // Grapheme gaps are empty and only shift what follows.
    fragments.retain(|fragment| !fragment.range.is_empty());
    Some(InputLine::inline(row.text.clone(), fragments))
}
