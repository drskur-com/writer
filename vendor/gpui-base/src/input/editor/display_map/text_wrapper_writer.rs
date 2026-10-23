//! Writer patches to the wrapper (see `docs/patches/`): per-line styles ([`LineStyle`]), blocks
//! below lines, collapsed lines and their reveal, and the row placement a line layout carries.

use super::*;

/// Per-line text size and row height, relative to the editor's font size and line height.
///
/// Lets a host enlarge whole lines (e.g. Markdown headings) without mixing font sizes inside a
/// line. Every visual row of the line uses the same scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineStyle {
    /// Multiplier for the editor font size.
    pub text_scale: f32,
    /// Multiplier for the editor line height (each wrapped row of the line).
    pub height_scale: f32,
    /// Height of a block painted below the line's last row. The measured block
    /// ([`LineBlockMeasureFn`]) is added to it, so a line style can reserve room below the
    /// measured block, e.g. for a page break mark the block renderer draws in the rest.
    pub block_height: Pixels,
    /// Hide the line's text (its rows take no height) unless a selection touches the line,
    /// e.g. to show only the block of a Markdown image line until the caret enters it.
    pub collapsed: bool,
    /// With [`Self::collapsed`]: a selection touching any line of a run of adjacent lines that
    /// all set this reveals the whole run, e.g. every row of a Markdown table.
    pub reveal_run: bool,
    /// With [`Self::reveal_run`]: this line starts a new run, so a run above it does not
    /// continue into it, e.g. the first row of a table right after another block.
    pub run_start: bool,
    /// Empty space below the line (after its block), e.g. the gap between paragraphs.
    pub space_after: Pixels,
    /// Height of a block painted above the line. Only the first buffer line's is used: it is
    /// a header above the document that scrolls with it (the block renderer gets
    /// [`crate::input::LineBlockContext::above`]), e.g. a page break mark before a title.
    pub block_before: Pixels,
    /// Extra left offset of the line's first visual row, e.g. a paragraph's first-line indent.
    /// The first row wraps that much narrower.
    pub first_indent: Pixels,
    /// How each visual row sits in the wrap width.
    pub align: LineAlign,
}

/// Horizontal placement of a line's visual rows within the wrap width (see [`LineStyle::align`]).
///
/// Only applies while soft wrapping, where the wrap width gives the room to align in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineAlign {
    #[default]
    Start,
    Center,
    End,
    /// Stretch every row but the last to the wrap width, widening the gaps between words
    /// (or between graphemes when a row has no spaces).
    Justify,
}

impl Default for LineStyle {
    fn default() -> Self {
        Self {
            text_scale: 1.,
            height_scale: 1.,
            block_height: px(0.),
            collapsed: false,
            reveal_run: false,
            run_start: false,
            space_after: px(0.),
            block_before: px(0.),
            first_indent: px(0.),
            align: LineAlign::Start,
        }
    }
}

impl LineStyle {
    #[inline]
    pub(crate) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A buffer line and its neighbors (all without the trailing `\n`), from which a host decides
/// the line's [`LineStyle`].
///
/// The neighbors let a style depend on the surrounding block structure, e.g. a blank line
/// between two paragraphs. When a line changes, its neighbors are styled again too.
#[derive(Debug, Clone, Copy)]
pub struct LineContext<'a> {
    /// The buffer row of the line, for styles that depend on document state the host tracks
    /// (call [`crate::input::EditorState::restyle_lines`] when that state changes).
    pub row: usize,
    pub text: &'a str,
    pub prev: Option<&'a str>,
    pub next: Option<&'a str>,
}

/// Decides the [`LineStyle`] of a buffer line from its text and neighbors.
pub type LineStyleFn = Rc<dyn Fn(LineContext<'_>) -> LineStyle>;

/// Decides the height of the block below a buffer line from its row, its text (without the
/// trailing `\n`) and the wrap width (`None` when not wrapping). Zero means no block.
///
/// The row is that of the text being wrapped, so a host can look up state it keeps per line,
/// e.g. the table a line ends.
///
/// Lets a host show content under a line, e.g. the image of a Markdown image line, while the
/// line itself stays editable text.
pub type LineBlockMeasureFn = Rc<dyn Fn(usize, &str, Option<Pixels>) -> Pixels>;

impl LineItem {
    /// Whether the line's text is hidden (see [`LineStyle::collapsed`]).
    #[inline]
    pub(crate) fn is_collapsed(&self) -> bool {
        self.style.collapsed && !self.revealed
    }

    /// Row height multiplier, zero while the line is collapsed.
    #[inline]
    pub(crate) fn height_scale(&self) -> f32 {
        if self.is_collapsed() {
            0.
        } else {
            self.style.height_scale
        }
    }

    /// Height reserved below the line's last row: its block and the space after it.
    #[inline]
    pub(crate) fn block_height(&self) -> Pixels {
        self.style.block_height + self.style.space_after
    }
}

impl TextWrapper {
    /// Set how each line is sized, and re-wrap every line.
    pub(crate) fn set_line_style(&mut self, line_style: Option<LineStyleFn>, cx: &mut App) {
        self.line_style = line_style;
        if self._initialized {
            self.update_all(&self.text.clone(), cx);
        }
    }

    /// Set how tall the block below each line is, and re-wrap every line.
    pub(crate) fn set_line_block(&mut self, line_block: Option<LineBlockMeasureFn>, cx: &mut App) {
        self.line_block = line_block;
        if self._initialized {
            self.update_all(&self.text.clone(), cx);
        }
    }

    /// Show the text of collapsed lines within `rows` (buffer rows touched by a selection) and
    /// hide the others. Returns whether any line changed.
    pub(crate) fn set_reveal(&mut self, rows: Vec<Range<usize>>, cx: &mut App) -> bool {
        self.reveal = rows
            .into_iter()
            .map(|rows| self.reveal_runs(rows))
            .collect();
        let reveal = &self.reveal;
        let changed: Vec<usize> = self
            .lines
            .iter()
            .enumerate()
            .filter(|(row, item)| {
                let revealed = item.style.collapsed && reveal.iter().any(|r| r.contains(row));
                item.revealed != revealed
            })
            .map(|(row, _)| row)
            .collect();
        let text = self.text.clone();
        for &row in &changed {
            let start = text.line_start_offset(row);
            let end = text.line_end_offset(row);
            self.update(
                &text,
                &(start..end),
                &Rope::from(text.slice(start..end).to_string()),
                cx,
            );
        }
        !changed.is_empty()
    }

    /// Widen `rows` over the runs of [`LineStyle::reveal_run`] lines they touch.
    fn reveal_runs(&self, rows: Range<usize>) -> Range<usize> {
        let in_run = |row: usize| {
            self.line(row)
                .is_some_and(|item| item.style.collapsed && item.style.reveal_run)
        };
        let starts_run = |row: usize| self.line(row).is_some_and(|item| item.style.run_start);
        let (mut start, mut end) = (rows.start, rows.end);
        if in_run(start) {
            while start > 0 && !starts_run(start) && in_run(start - 1) {
                start -= 1;
            }
        }
        if end > start && in_run(end - 1) {
            while in_run(end) && !starts_run(end) {
                end += 1;
            }
        }
        start..end
    }

    /// Style and re-wrap the lines in `rows` (buffer rows) again, e.g. after host state that the
    /// line style reads has changed.
    pub(crate) fn restyle_rows(&mut self, rows: Range<usize>, cx: &mut App) {
        let count = self.lines_count();
        let rows = rows.start.min(count)..rows.end.min(count);
        if self.line_style.is_none() || rows.is_empty() {
            return;
        }
        let text = self.text.clone();
        let start = text.line_start_offset(rows.start);
        let end = text.line_end_offset(rows.end - 1);
        self.update(
            &text,
            &(start..end),
            &Rope::from(text.slice(start..end).to_string()),
            cx,
        );
    }

    /// How tall the block below each line is.
    pub(crate) fn line_block(&self) -> Option<LineBlockMeasureFn> {
        self.line_block.clone()
    }

    /// Whether any line has a non-default [`LineStyle`].
    #[inline]
    pub(crate) fn has_styled_lines(&self) -> bool {
        self.lines.summary().styled_lines > 0
    }
}

impl LineLayout {
    /// Set the left offset of each visual row (see [`Self::line_indent`]).
    pub(crate) fn row_offsets(mut self, row_offsets: SmallVec<[Pixels; 1]>) -> Self {
        self.row_offsets = row_offsets;
        self
    }

    /// Mark the bottom `block_space` of the block as empty space.
    pub(crate) fn block_space(mut self, block_space: Pixels) -> Self {
        self.block_space = block_space;
        self
    }

    /// Set the height of each visual row, overriding the editor line height.
    pub(crate) fn row_height(mut self, row_height: Option<Pixels>) -> Self {
        self.row_height = row_height;
        self
    }

    /// Do not paint the text (a collapsed line).
    pub(crate) fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// Whether the text is not painted (a collapsed line).
    pub(crate) fn is_hidden(&self) -> bool {
        self.hidden
    }

    /// Reserve a block of `block_height` below the last visual row.
    pub(crate) fn block_height(mut self, block_height: Pixels) -> Self {
        self.block_height = block_height;
        self
    }

    /// Height of each visual row of this line.
    #[inline]
    pub(crate) fn row_height_or(&self, line_height: Pixels) -> Pixels {
        self.row_height.unwrap_or(line_height)
    }
}
