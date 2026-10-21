/// DisplayMap: Public facade for Editor/Input display mapping.
///
/// This combines WrapMap and FoldMap to provide a unified API:
/// - BufferPoint ↔ DisplayPoint conversion
/// - Fold management (candidates, toggle, query)
/// - Automatic projection updates on text/layout changes
use std::ops::Range;

use gpui::{App, Font, Pixels, px};
use ropey::Rope;

use super::fold_map::FoldMap;
use super::folding::FoldRange;
pub use super::text_wrapper::WrappingIndent;
use super::text_wrapper::{LineBlockMeasureFn, LineItem, LineStyle, LineStyleFn, WrapDisplayPoint};
use super::wrap_map::WrapMap;
use super::{BufferPoint, DisplayPoint};
use crate::input::Point as TreeSitterPoint;
use crate::input::display_map::WrapPoint;
use crate::input::rope_ext::RopeExt as _;

/// DisplayMap is the main interface for Editor/Input coordinate mapping.
///
/// It manages the two-layer projection:
/// 1. Buffer → Wrap (soft-wrapping)
/// 2. Wrap → Display (folding)
///
/// Editor/Input only needs to work with BufferPoint and DisplayPoint.
pub struct DisplayMap {
    wrap_map: WrapMap,
    fold_map: FoldMap,
    /// Display rows taller than the line height, sorted by row. Empty when every line has the
    /// default style.
    tall_rows: Vec<(usize, TallRow)>,
    /// `tall_rows_prefix[i]` = sum of `tall_rows[..i]`.
    tall_rows_prefix: Vec<TallRow>,
    /// Space above the first row: the first line's [`LineStyle::block_before`].
    top_inset: Pixels,
}

/// Height of a display row beyond one line height.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct TallRow {
    /// In line heights, from [`LineStyle::height_scale`].
    rows: f32,
    /// The block below a line's last row ([`LineStyle::block_height`]).
    block: Pixels,
}

impl TallRow {
    fn height(&self, line_height: Pixels) -> Pixels {
        line_height * self.rows + self.block
    }
}

impl DisplayMap {
    pub fn new(font: Font, font_size: Pixels, wrap_width: Option<Pixels>) -> Self {
        Self {
            wrap_map: WrapMap::new(font, font_size, wrap_width),
            fold_map: FoldMap::new(),
            tall_rows: Vec::new(),
            tall_rows_prefix: Vec::new(),
            top_inset: px(0.),
        }
    }

    // ==================== Core Coordinate Mapping ====================

    /// Convert buffer position to display position
    pub fn buffer_pos_to_display_pos(&self, pos: BufferPoint) -> DisplayPoint {
        // Buffer → Wrap
        let wrap_pos = self.wrap_map.buffer_pos_to_wrap_pos(pos);

        // Wrap → Display
        if let Some(display_row) = self.fold_map.wrap_row_to_display_row(wrap_pos.row) {
            DisplayPoint::new(display_row, wrap_pos.col)
        } else {
            // Cursor is in a folded region, find nearest visible row
            let display_row = self.fold_map.nearest_visible_display_row(wrap_pos.row);
            DisplayPoint::new(display_row, 0) // Column 0 at fold boundary
        }
    }

    /// Convert display position to buffer position
    pub fn display_pos_to_buffer_pos(&self, pos: DisplayPoint) -> BufferPoint {
        // Display → Wrap
        let wrap_row = self.fold_map.display_row_to_wrap_row(pos.row).unwrap_or(0);

        // Wrap → Buffer
        let wrap_pos = WrapPoint::new(wrap_row, pos.col);
        self.wrap_map.wrap_pos_to_buffer_pos(wrap_pos)
    }

    /// Get total number of visible display rows
    #[inline]
    pub fn display_row_count(&self) -> usize {
        self.fold_map.display_row_count()
    }

    /// Get the buffer line for a given display row
    pub fn display_row_to_buffer_line(&self, display_row: usize) -> usize {
        // Display → Wrap
        let wrap_row = self
            .fold_map
            .display_row_to_wrap_row(display_row)
            .unwrap_or(0);

        // Wrap → Buffer line
        self.wrap_map.wrap_row_to_buffer_line(wrap_row)
    }

    /// Get the display row range for a buffer line: [start, end)
    /// Returns None if the buffer line is completely hidden
    pub fn buffer_line_to_display_row_range(&self, line: usize) -> Option<Range<usize>> {
        // Buffer line → Wrap row range
        let wrap_row_range = self.wrap_map.buffer_line_to_wrap_row_range(line);

        // Find first and last visible display rows in this range
        let mut first_display_row = None;
        let mut last_display_row = None;

        for wrap_row in wrap_row_range {
            if let Some(display_row) = self.fold_map.wrap_row_to_display_row(wrap_row) {
                if first_display_row.is_none() {
                    first_display_row = Some(display_row);
                }
                last_display_row = Some(display_row);
            }
        }

        if let (Some(start), Some(end)) = (first_display_row, last_display_row) {
            Some(start..end + 1)
        } else {
            None // Completely folded
        }
    }

    /// Check if a buffer line is completely hidden
    #[inline]
    pub fn is_buffer_line_hidden(&self, line: usize) -> bool {
        self.buffer_line_to_display_row_range(line).is_none()
    }

    /// First display row of a buffer line. If the line is fully folded, returns the
    /// nearest visible display row.
    pub fn buffer_line_to_display_row(&self, line: usize) -> usize {
        match self.buffer_line_to_display_row_range(line) {
            Some(range) => range.start,
            None => {
                let wrap_row = self.wrap_map.buffer_line_to_first_wrap_row(line);
                self.fold_map.nearest_visible_display_row(wrap_row)
            }
        }
    }

    /// Set fold candidates (from tree-sitter/LSP)
    pub fn set_fold_candidates(&mut self, candidates: Vec<FoldRange>) {
        self.fold_map.set_candidates(candidates);
        self.rebuild_fold_projection();
    }

    /// Set a fold at the given start_line (must be in candidates)
    pub fn set_folded(&mut self, start_line: usize, folded: bool) {
        self.fold_map.set_folded(start_line, folded);
        self.rebuild_fold_projection();
    }

    /// Toggle fold at the given start_line
    pub fn toggle_fold(&mut self, start_line: usize) {
        self.fold_map.toggle_fold(start_line);
        self.rebuild_fold_projection();
    }

    /// Check if a line is currently folded
    #[inline]
    pub fn is_folded_at(&self, start_line: usize) -> bool {
        self.fold_map.is_folded_at(start_line)
    }

    /// Check if a line is a fold candidate
    #[inline]
    pub fn is_fold_candidate(&self, start_line: usize) -> bool {
        self.fold_map.is_fold_candidate(start_line)
    }

    /// Get all currently folded ranges
    #[inline]
    pub fn folded_ranges(&self) -> &[FoldRange] {
        self.fold_map.folded_ranges()
    }

    /// Clear all folds
    pub fn clear_folds(&mut self) {
        self.fold_map.clear_folds();
        self.rebuild_fold_projection();
    }

    // ==================== Text and Layout Updates ====================

    /// Adjust folds and candidates for a text edit before updating the wrap map.
    ///
    /// Must be called with the OLD text (before replacement) and the edit range/new_text
    /// so we can compute which old lines were affected.
    pub fn adjust_folds_for_edit(&mut self, old_text: &Rope, range: &Range<usize>, new_text: &str) {
        if self.fold_map.folded_ranges().is_empty() && self.fold_map.fold_candidates().is_empty() {
            return;
        }

        let edit_start_line = old_text.offset_to_point(range.start).row;
        let edit_end_line = old_text.offset_to_point(range.end.min(old_text.len())).row;

        let old_lines_in_range = edit_end_line.saturating_sub(edit_start_line);
        let new_lines_in_range = new_text.chars().filter(|c| *c == '\n').count();
        let line_delta = new_lines_in_range as isize - old_lines_in_range as isize;

        self.fold_map
            .adjust_folds_for_edit(edit_start_line, edit_end_line, line_delta);
    }

    /// Incrementally update fold candidates after a text edit.
    ///
    /// Extracts new fold candidates only within the edited byte range
    /// and merges them with existing (already adjusted) candidates.
    pub fn update_fold_candidates_for_edit(
        &mut self,
        extract_fold_ranges: impl FnOnce(Range<usize>, &Rope) -> Vec<FoldRange>,
        edit_byte_range: Range<usize>,
        new_text: &Rope,
    ) {
        let new_start_line = new_text.offset_to_point(edit_byte_range.start).row;
        let new_end_line = new_text
            .offset_to_point(edit_byte_range.end.min(new_text.len()))
            .row;

        let new_candidates = extract_fold_ranges(edit_byte_range, new_text);
        self.fold_map
            .merge_candidates_for_edit(new_start_line, new_end_line, new_candidates);
    }

    /// Update text (incremental or full)
    pub fn on_text_changed(
        &mut self,
        changed_text: &Rope,
        range: &Range<usize>,
        new_text: &Rope,
        cx: &mut App,
    ) {
        self.wrap_map
            .on_text_changed(changed_text, range, new_text, cx);
        self.rebuild_fold_projection();
    }

    /// Update layout parameters (wrap width or font)
    pub(crate) fn set_inline_metrics(
        &mut self,
        metrics: std::rc::Rc<[(std::ops::Range<usize>, Pixels)]>,
        cx: &mut App,
    ) {
        self.wrap_map.set_inline_metrics(metrics, cx);
        self.rebuild_fold_projection();
    }

    pub fn on_layout_changed(&mut self, wrap_width: Option<Pixels>, cx: &mut App) {
        self.wrap_map.on_layout_changed(wrap_width, cx);
        self.rebuild_fold_projection();
    }

    /// Set the wrapping indent for continuation lines.
    pub fn set_wrapping_indent(&mut self, wrapping_indent: WrappingIndent, cx: &mut App) {
        self.wrap_map.set_wrapping_indent(wrapping_indent, cx);
        self.rebuild_fold_projection();
    }

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
        let ix = self.tall_rows.partition_point(|&(row, _)| row < display_row);
        self.tall_rows_prefix.get(ix).copied().unwrap_or_default()
    }

    /// Extra height of `display_row`.
    fn extra_of(&self, display_row: usize) -> TallRow {
        match self.tall_rows.binary_search_by_key(&display_row, |&(row, _)| row) {
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
    fn rebuild_tall_rows(&mut self) {
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
                        block: if row == last {
                            block_height
                        } else {
                            px(0.)
                        },
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

    /// Set font parameters
    pub fn set_font(&mut self, font: Font, font_size: Pixels, cx: &mut App) {
        self.wrap_map.set_font(font, font_size, cx);
        self.rebuild_fold_projection();
    }

    /// Ensure text is prepared (initializes wrapper if needed)
    pub fn ensure_text_prepared(&mut self, text: &Rope, cx: &mut App) {
        let did_initialize = self.wrap_map.ensure_text_prepared(text, cx);
        if did_initialize {
            self.rebuild_fold_projection();
        }
    }

    /// Initialize with text
    pub fn set_text(&mut self, text: &Rope, cx: &mut App) {
        self.wrap_map.set_text(text, cx);
        self.rebuild_fold_projection();
    }

    // ==================== Internal Helpers ====================

    /// Rebuild fold projection after wrap_map or fold state changes
    /// Only rebuilds if there are actually folded ranges
    fn rebuild_fold_projection(&mut self) {
        if !self.fold_map.folded_ranges().is_empty() {
            self.fold_map.rebuild(&self.wrap_map);
        } else {
            // No active folds: identity mapping (wrap_row == display_row).
            // Just update cached count so query methods work without Vec allocation.
            self.fold_map
                .mark_dirty_with_wrap_count(self.wrap_map.wrap_row_count());
        }
        self.rebuild_tall_rows();
    }

    // ==================== Wrap Display Point Operations ====================

    /// Convert byte offset to wrap display point (with soft wrap info).
    #[inline]
    pub(crate) fn offset_to_wrap_display_point(&self, offset: usize) -> WrapDisplayPoint {
        self.wrap_map.wrapper().offset_to_display_point(offset)
    }

    /// Like [`Self::offset_to_wrap_display_point`], but honours the caret's line-end affinity so
    /// an offset on a soft wrap boundary resolves to the row the caret is drawn on.
    #[inline]
    pub(crate) fn offset_to_wrap_display_point_with_affinity(
        &self,
        offset: usize,
        line_end_affinity: bool,
    ) -> WrapDisplayPoint {
        self.wrap_map
            .wrapper()
            .offset_to_display_point_with_affinity(offset, line_end_affinity)
    }

    /// Resolve the caret's visual row through both wrapping and folded regions.
    pub(crate) fn offset_to_display_row(&self, offset: usize, line_end_affinity: bool) -> usize {
        let point = self.offset_to_wrap_display_point_with_affinity(offset, line_end_affinity);
        self.wrap_row_to_display_row(point.row)
            .unwrap_or_else(|| self.nearest_visible_display_row(point.row))
    }

    /// Convert wrap display point to byte offset.
    #[inline]
    pub(crate) fn wrap_display_point_to_offset(&self, point: WrapDisplayPoint) -> usize {
        self.wrap_map.wrapper().display_point_to_offset(point)
    }

    /// Convert wrap display point to TreeSitterPoint (buffer line/col).
    #[inline]
    pub(crate) fn wrap_display_point_to_point(&self, point: WrapDisplayPoint) -> TreeSitterPoint {
        self.wrap_map.wrapper().display_point_to_point(point)
    }

    /// Convert a wrap row to a display row (skipping folded rows).
    /// Returns None if the wrap row is folded.
    #[inline]
    pub fn wrap_row_to_display_row(&self, wrap_row: usize) -> Option<usize> {
        self.fold_map.wrap_row_to_display_row(wrap_row)
    }

    /// Find the nearest visible display row for a given wrap row.
    #[inline]
    pub fn nearest_visible_display_row(&self, wrap_row: usize) -> usize {
        self.fold_map.nearest_visible_display_row(wrap_row)
    }

    /// Convert a display row to a wrap row.
    #[inline]
    pub fn display_row_to_wrap_row(&self, display_row: usize) -> Option<usize> {
        self.fold_map.display_row_to_wrap_row(display_row)
    }

    pub(crate) fn display_row_column_to_offset(&self, display_row: usize, column: usize) -> usize {
        let wrap_row = self
            .fold_map
            .display_row_to_wrap_row(display_row)
            .unwrap_or(0);
        self.wrap_map
            .wrapper()
            .display_point_to_offset(WrapDisplayPoint::new(wrap_row, 0, column))
    }

    /// Get the longest row index (by byte length).
    #[inline]
    pub(crate) fn longest_row(&self) -> usize {
        self.wrap_map.wrapper().longest_row()
    }

    // ==================== Access Methods ====================

    /// Get the line item by buffer row index.
    #[inline]
    pub(crate) fn line(&self, row: usize) -> Option<&LineItem> {
        self.wrap_map.line(row)
    }

    /// Get the rope text
    #[inline]
    pub fn text(&self) -> &Rope {
        self.wrap_map.text()
    }

    /// Calculate how many wrap rows of a buffer line are visible (not folded)
    #[inline]
    pub fn visible_wrap_row_count_for_buffer_line(&self, line: usize) -> usize {
        self.wrap_map
            .visible_wrap_row_count_for_line(line, &self.fold_map)
    }

    /// Get the wrap row count (before folding)
    #[inline]
    pub fn wrap_row_count(&self) -> usize {
        self.wrap_map.wrap_row_count()
    }

    /// Get the buffer line count (logical lines)
    #[inline]
    pub fn buffer_line_count(&self) -> usize {
        self.wrap_map.buffer_line_count()
    }
}
