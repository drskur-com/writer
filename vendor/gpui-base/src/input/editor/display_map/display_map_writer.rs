//! Writer patches to the display map (see `docs/patches/`): per-line styles, blocks and collapsed
//! lines, and the vertical geometry of rows that are taller than the line height.

use super::*;
use gpui::px;

use super::super::text_wrapper::{LineBlockMeasureFn, LineStyle, LineStyleFn};

/// Height of a display row beyond one line height.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct TallRow {
    /// In line heights, from [`LineStyle::height_scale`].
    pub(super) rows: f32,
    /// The block below a line's last row ([`LineStyle::block_height`]).
    pub(super) block: Pixels,
}

impl TallRow {
    pub(super) fn height(&self, line_height: Pixels) -> Pixels {
        line_height * self.rows + self.block
    }
}

impl DisplayMap {
    /// Set how each line is sized (see [`LineStyle`]); `None` restores uniform lines.
    pub fn set_line_style(&mut self, line_style: Option<LineStyleFn>, cx: &mut App) {
        self.wrap_map.set_line_style(line_style, cx);
        self.rebuild_fold_projection();
    }

    /// Style the lines in `rows` (buffer rows) again with the current line style.
    pub fn restyle_rows(&mut self, rows: Range<usize>, cx: &mut App) {
        self.wrap_map.restyle_rows(rows, cx);
        self.rebuild_fold_projection();
    }

    /// Set how tall the block below each line is (see [`LineBlockMeasureFn`]); `None` removes
    /// every block.
    pub fn set_line_block(&mut self, line_block: Option<LineBlockMeasureFn>, cx: &mut App) {
        self.wrap_map.set_line_block(line_block, cx);
        self.rebuild_fold_projection();
    }

    /// Show the text of collapsed lines (see [`LineStyle::collapsed`]) within `rows`, the buffer
    /// rows touched by a selection, and hide the others.
    pub fn set_revealed_rows(&mut self, rows: Vec<Range<usize>>, cx: &mut App) {
        if self.wrap_map.set_reveal(rows, cx) {
            self.rebuild_fold_projection();
        }
    }

    /// Whether any line has a non-default [`LineStyle`].
    pub fn has_styled_lines(&self) -> bool {
        self.wrap_map.wrapper().has_styled_lines()
    }

    /// How tall the block below each line is, if set.
    pub fn line_block(&self) -> Option<LineBlockMeasureFn> {
        self.wrap_map.line_block()
    }

    /// Style of a buffer line.
    pub fn line_style(&self, line: usize) -> LineStyle {
        self.wrap_map
            .line(line)
            .map_or_else(LineStyle::default, |item| item.style)
    }

    // ==================== Vertical Geometry ====================

    /// Extra height of the tall rows before `display_row`.
    fn extra_before(&self, display_row: usize) -> TallRow {
        let ix = self
            .tall_rows
            .partition_point(|&(row, _)| row < display_row);
        self.tall_rows_prefix.get(ix).copied().unwrap_or_default()
    }

    /// Extra height of `display_row`.
    fn extra_of(&self, display_row: usize) -> TallRow {
        match self
            .tall_rows
            .binary_search_by_key(&display_row, |&(row, _)| row)
        {
            Ok(ix) => self.tall_rows[ix].1,
            Err(_) => TallRow::default(),
        }
    }

    /// Top of a display row in content space. `display_row` may equal the row count, giving
    /// the total content height.
    pub fn display_row_top(&self, display_row: usize, line_height: Pixels) -> Pixels {
        self.top_inset
            + line_height * display_row as f32
            + self.extra_before(display_row).height(line_height)
    }

    /// Space above the first row (see [`LineStyle::block_before`]).
    pub fn top_inset(&self) -> Pixels {
        self.top_inset
    }

    /// Height of a display row, including a line block below it.
    pub fn display_row_height(&self, display_row: usize, line_height: Pixels) -> Pixels {
        line_height + self.extra_of(display_row).height(line_height)
    }

    /// Height of a display row's text, without a line block below it.
    pub fn display_row_text_height(&self, display_row: usize, line_height: Pixels) -> Pixels {
        line_height * (1. + self.extra_of(display_row).rows)
    }

    /// Total height of all display rows.
    pub fn content_height(&self, line_height: Pixels) -> Pixels {
        self.display_row_top(self.display_row_count(), line_height)
    }

    /// Display row under content-space `y`, clamped to the existing rows.
    pub fn display_row_at_y(&self, y: Pixels, line_height: Pixels) -> usize {
        let count = self.display_row_count();
        if count == 0 || line_height <= Pixels::ZERO {
            return 0;
        }
        if self.tall_rows.is_empty() {
            return (((y - self.top_inset) / line_height).max(0.) as usize).min(count - 1);
        }
        // Last row whose top is at or above `y`.
        let (mut low, mut high) = (0, count);
        while low + 1 < high {
            let mid = (low + high) / 2;
            if self.display_row_top(mid, line_height) <= y {
                low = mid;
            } else {
                high = mid;
            }
        }
        low
    }

    /// Recompute [`Self::tall_rows`] after wrap, fold or style changes.
    pub(super) fn rebuild_tall_rows(&mut self) {
        self.tall_rows.clear();
        self.tall_rows_prefix.clear();
        self.top_inset = self.line_style(0).block_before.max(px(0.));
        if !self.wrap_map.wrapper().has_styled_lines() {
            return;
        }
        let styled: Vec<(usize, f32, Pixels)> = self
            .wrap_map
            .wrapper()
            .iter_lines()
            .enumerate()
            .filter(|(_, item)| item.height_scale() != 1. || item.block_height() > px(0.))
            .map(|(line, item)| (line, item.height_scale(), item.block_height()))
            .collect();
        for (line, height_scale, block_height) in styled {
            if let Some(rows) = self.buffer_line_to_display_row_range(line) {
                let last = rows.end.saturating_sub(1);
                self.tall_rows.extend(rows.filter_map(|row| {
                    let extra = TallRow {
                        rows: height_scale - 1.,
                        // The block sits below the line's last row.
                        block: if row == last { block_height } else { px(0.) },
                    };
                    (extra != TallRow::default()).then_some((row, extra))
                }));
            }
        }
        let mut sum = TallRow::default();
        self.tall_rows_prefix.reserve(self.tall_rows.len() + 1);
        for &(_, extra) in &self.tall_rows {
            self.tall_rows_prefix.push(sum);
            sum.rows += extra.rows;
            sum.block += extra.block;
        }
        self.tall_rows_prefix.push(sum);
    }
}
