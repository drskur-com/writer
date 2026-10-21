//! Writer patches to the editor (see `docs/patches/`): line styles, blocks below lines, overlays on
//! parts of lines, and an edit that keeps the selections.

use gpui::{App, Window};

use crate::input::{EditorMode, InputBaseState};

impl InputBaseState<EditorMode> {
    /// Size and place whole lines from their text, e.g. to enlarge Markdown headings or indent
    /// and justify paragraphs.
    ///
    /// `line_style` gets each buffer line and its neighbors without their trailing `\n`
    /// ([`crate::input::LineContext`]) and returns its [`crate::input::LineStyle`]. It is called again for
    /// every edited line and the lines next to it, so it should be cheap and depend only on
    /// that context. Pass `None` to make every line uniform again.
    pub fn set_line_style(
        &mut self,
        line_style: Option<
            impl Fn(crate::input::LineContext<'_>) -> crate::input::LineStyle + 'static,
        >,
        cx: &mut gpui::Context<Self>,
    ) {
        let line_style =
            line_style.map(|line_style| std::rc::Rc::new(line_style) as crate::input::LineStyleFn);
        self.display_map.set_line_style(line_style, cx);
        cx.notify();
    }

    /// Style the buffer lines in `rows` again with the current line style, e.g. when state the
    /// style reads (through [`crate::input::LineContext::row`]) changed without an edit on those lines.
    pub fn restyle_lines(&mut self, rows: std::ops::Range<usize>, cx: &mut gpui::Context<Self>) {
        self.display_map.restyle_rows(rows, cx);
        cx.notify();
    }

    /// Reserve a block below some lines and draw into it, e.g. the image of a Markdown image
    /// line. The line itself stays ordinary editable text above its block.
    ///
    /// `measure` gets each buffer row, its line without the trailing `\n` and the wrap width, and returns
    /// the block height (zero for no block). Like `line_style`, it is called again for every
    /// edited line and on every re-wrap, so it should be cheap. `render` draws a visible block;
    /// clicks on it put the caret on its line.
    pub fn set_line_blocks(
        &mut self,
        measure: impl Fn(usize, &str, Option<gpui::Pixels>) -> gpui::Pixels + 'static,
        render: impl Fn(&LineBlockContext, &mut Window, &mut App) -> gpui::AnyElement + 'static,
        cx: &mut gpui::Context<Self>,
    ) {
        self.line_block_renderer = Some(std::rc::Rc::new(render));
        self.display_map.set_line_block(
            Some(std::rc::Rc::new(measure) as crate::input::LineBlockMeasureFn),
            cx,
        );
        cx.notify();
    }

    /// Remove every line block (see [`Self::set_line_blocks`]).
    pub fn clear_line_blocks(&mut self, cx: &mut gpui::Context<Self>) {
        self.line_block_renderer = None;
        self.display_map.set_line_block(None, cx);
        cx.notify();
    }

    /// Draw elements over parts of lines, e.g. a check box over the `- [ ] ` of a Markdown task
    /// item. The text stays in the document; the overlay only covers it, so it should paint an
    /// opaque background.
    ///
    /// `ranges` gets each visible buffer row and its line without the trailing `\n`, and returns
    /// the byte ranges (within the line) to cover. It runs on every frame for the visible lines,
    /// so it should be cheap. A range must sit on one visual row; others are skipped. Lines that
    /// a selection (or the caret) touches show their text instead, so it can be edited.
    ///
    /// `render` draws one overlay at the size of the text it covers. Its mouse-down handler can
    /// call `window.prevent_default()` to keep the press from placing the caret.
    pub fn set_line_overlays(
        &mut self,
        ranges: impl Fn(usize, &str) -> Vec<std::ops::Range<usize>> + 'static,
        render: impl Fn(&LineOverlayContext, &mut Window, &mut App) -> gpui::AnyElement + 'static,
        cx: &mut gpui::Context<Self>,
    ) {
        self.line_overlays = Some((std::rc::Rc::new(ranges), std::rc::Rc::new(render)));
        cx.notify();
    }

    /// Remove every line overlay (see [`Self::set_line_overlays`]).
    pub fn clear_line_overlays(&mut self, cx: &mut gpui::Context<Self>) {
        self.line_overlays = None;
        cx.notify();
    }

    /// Replace the text in `range` (bytes) as one undoable edit and keep the selections where
    /// they are (shifted past the edit), e.g. to tick a task item's check box without moving
    /// the caret to it.
    pub fn replace_range_keeping_selections(
        &mut self,
        range: std::ops::Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let range = range.start.min(self.text.len())..range.end.min(self.text.len());
        let delta = text.len() as isize - range.len() as isize;
        let shift = |offset: usize| {
            if offset >= range.end {
                (offset as isize + delta) as usize
            } else {
                offset.min(range.start + text.len())
            }
        };
        let kept: Vec<_> = self
            .selections
            .iter()
            .map(|selection| {
                let mut selection = *selection;
                selection.start = shift(selection.start);
                selection.end = shift(selection.end);
                selection
            })
            .collect();
        self.with_edits_allowed(|this| {
            this.undo_manager
                .set_pending_intent(crate::input::undo_manager::EditIntent::Atomic);
            this.replace_text_in_ranges(&[(range.clone(), text.to_string())], window, cx);
        });
        self.selections.replace_all(kept);
        cx.notify();
    }

    /// Measure the line blocks again, e.g. after what they show finished loading.
    pub fn refresh_line_blocks(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(measure) = self.display_map.line_block() {
            self.display_map.set_line_block(Some(measure), cx);
            cx.notify();
        }
    }
}

/// A block below a line, for a [`LineBlockRenderer`].
pub struct LineBlockContext {
    /// Buffer line the block belongs to.
    pub line: usize,
    /// Text of the line, without the trailing `\n`.
    pub text: gpui::SharedString,
    /// Size of the block: the text width and the measured block height.
    pub size: gpui::Size<gpui::Pixels>,
    /// The block sits above the line (the first line's [`crate::input::LineStyle::block_before`])
    /// rather than below it.
    pub above: bool,
}

/// Draws the block below a line (see [`EditorState::set_line_blocks`]).
pub type LineBlockRenderer =
    std::rc::Rc<dyn Fn(&LineBlockContext, &mut Window, &mut App) -> gpui::AnyElement>;

/// An overlay on part of a line, for a [`LineOverlayRenderer`].
pub struct LineOverlayContext {
    /// Buffer line the overlay sits on.
    pub line: usize,
    /// Text of the line, without the trailing `\n`.
    pub text: gpui::SharedString,
    /// Byte range within the line that the overlay covers.
    pub range: std::ops::Range<usize>,
    /// Size of the covered text: its width and the height of its visual row.
    pub size: gpui::Size<gpui::Pixels>,
}

/// Finds the parts of a line to cover (see [`EditorState::set_line_overlays`]).
pub type LineOverlayRanges = std::rc::Rc<dyn Fn(usize, &str) -> Vec<std::ops::Range<usize>>>;

/// Draws an overlay on part of a line (see [`EditorState::set_line_overlays`]).
pub type LineOverlayRenderer =
    std::rc::Rc<dyn Fn(&LineOverlayContext, &mut Window, &mut App) -> gpui::AnyElement>;
