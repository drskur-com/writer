//! Writer patches to the wrap map (see `docs/patches/`): passes line styles, blocks and reveal
//! through to the wrapper.

use super::super::text_wrapper::{LineBlockMeasureFn, LineStyleFn};
use super::*;

impl WrapMap {
    /// Set how each line is sized (see [`LineStyle`]).
    pub(in crate::input::display_map) fn set_line_style(
        &mut self,
        line_style: Option<LineStyleFn>,
        cx: &mut App,
    ) {
        self.wrapper.set_line_style(line_style, cx);
    }

    /// Style the lines in `rows` again (see [`TextWrapper::restyle_rows`]).
    pub(in crate::input::display_map) fn restyle_rows(
        &mut self,
        rows: std::ops::Range<usize>,
        cx: &mut App,
    ) {
        self.wrapper.restyle_rows(rows, cx);
    }

    /// Set how tall the block below each line is (see [`LineBlockMeasureFn`]).
    pub(in crate::input::display_map) fn set_line_block(
        &mut self,
        line_block: Option<LineBlockMeasureFn>,
        cx: &mut App,
    ) {
        self.wrapper.set_line_block(line_block, cx);
    }

    /// Show collapsed lines within `rows` (see [`TextWrapper::set_reveal`]).
    pub(in crate::input::display_map) fn set_reveal(
        &mut self,
        rows: Vec<std::ops::Range<usize>>,
        cx: &mut App,
    ) -> bool {
        self.wrapper.set_reveal(rows, cx)
    }

    /// How tall the block below each line is.
    pub(in crate::input::display_map) fn line_block(&self) -> Option<LineBlockMeasureFn> {
        self.wrapper.line_block()
    }
}
