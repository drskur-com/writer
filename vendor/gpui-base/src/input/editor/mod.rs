use gpui::{App, Div, Entity, InteractiveElement as _, IntoElement, RenderOnce, Stateful, Window};

use super::{EditorMode, InputBaseState, InputModeKind};

/// State for source-code editing.
///
/// This is the shared editing engine in its code-editor kind. Languages, line
/// numbers, folding, indent guides, diagnostics, decorations, and the LSP
/// providers exist on this kind only, so an ordinary input or textarea never
/// exposes them.
pub type EditorState = InputBaseState<EditorMode>;

impl InputModeKind for EditorMode {
    const MULTI_LINE: bool = true;
    const CODE_EDITOR: bool = true;

    type Extras = super::EditorExtras;

    fn hover_definition_style(
        state: &InputBaseState<Self>,
        _cx: &App,
    ) -> Option<(std::ops::Range<usize>, gpui::HighlightStyle)> {
        state.hover_definition_style()
    }

    fn hover_definition_hitbox(
        state: &InputBaseState<Self>,
        window: &mut Window,
        _cx: &App,
    ) -> Option<gpui::Hitbox> {
        state.hover_definition_hitbox(window)
    }

    fn reset_language_features(state: &mut InputBaseState<Self>) {
        state.extras.lsp.reset();
    }

    fn reset_annotations(state: &mut InputBaseState<Self>) {
        state.extras.hover_popover = None;
        state.extras.decorations.clear();
        state.extras.range_decorations.clear();
    }

    fn editing_syntax_context(state: &InputBaseState<Self>, offset: usize) -> super::SyntaxContext {
        state.syntax_context_at(offset)
    }

    fn adjust_annotations(
        state: &mut InputBaseState<Self>,
        range: &std::ops::Range<usize>,
        new_len: usize,
    ) {
        state.extras.decorations.adjust_for_edit(range, new_len);
        state
            .extras
            .range_decorations
            .adjust_for_edit(range, new_len);
    }

    fn refresh_language_features(
        state: &mut InputBaseState<Self>,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        let text = state.text().clone();
        state.extras.lsp.update(&text, window, cx);
    }

    fn accept_inline_completion(
        state: &mut InputBaseState<Self>,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) -> bool {
        state.accept_inline_completion(window, cx)
    }

    fn has_inline_completion(state: &InputBaseState<Self>) -> bool {
        state.has_inline_completion()
    }

    fn on_click(
        state: &mut InputBaseState<Self>,
        event: &gpui::MouseDownEvent,
        offset: usize,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) -> bool {
        state.handle_click_hover_definition(event, offset, window, cx)
    }

    fn clear_hover_state(
        state: &mut InputBaseState<Self>,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.clear_hover_state(cx);
    }

    fn on_text_typed(
        state: &mut InputBaseState<Self>,
        range: &std::ops::Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.handle_completion_trigger(range, text, window, cx);
    }

    fn clear_inline_completion(
        state: &mut InputBaseState<Self>,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.clear_inline_completion(cx);
    }

    fn hide_context_menu(
        state: &mut InputBaseState<Self>,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.hide_context_menu(cx);
    }

    fn is_context_menu_open(state: &InputBaseState<Self>, cx: &App) -> bool {
        state.is_context_menu_open(cx)
    }

    fn handle_context_menu_action(
        state: &mut InputBaseState<Self>,
        action: Box<dyn gpui::Action>,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) -> bool {
        state.handle_action_for_context_menu(action, window, cx)
    }

    fn on_hover_definition(
        state: &mut InputBaseState<Self>,
        offset: usize,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.handle_hover_definition(offset, window, cx);
    }

    fn on_mouse_move(
        state: &mut InputBaseState<Self>,
        offset: usize,
        event: &gpui::MouseMoveEvent,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        state.handle_mouse_move(offset, event, window, cx);
    }

    fn drive_highlighter(
        highlighter: &std::rc::Rc<std::cell::RefCell<Option<Box<dyn super::InputHighlighter>>>>,
        edit: super::InputEdit,
        text: &ropey::Rope,
        folding: bool,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        let mut highlighter = highlighter.borrow_mut();
        let Some(highlighter) = highlighter.as_mut() else {
            return;
        };
        highlighter.update(Some(edit), text, folding, window, cx);
    }

    fn drive_highlighter_batch(
        highlighter: &std::rc::Rc<std::cell::RefCell<Option<Box<dyn super::InputHighlighter>>>>,
        edits: &[(super::InputEdit, ropey::Rope)],
        folding: bool,
        window: &mut Window,
        cx: &mut gpui::Context<InputBaseState<Self>>,
    ) {
        let mut highlighter = highlighter.borrow_mut();
        let Some(highlighter) = highlighter.as_mut() else {
            return;
        };
        highlighter.update_batch(edits, folding, window, cx);
    }

    fn register_actions(
        element: Stateful<Div>,
        entity: &Entity<InputBaseState<Self>>,
        window: &mut Window,
    ) -> Stateful<Div> {
        element
            .key_context("Input mode=editor")
            .on_action(window.listener_for(entity, InputBaseState::on_action_toggle_code_actions))
            .on_action(window.listener_for(entity, InputBaseState::on_action_show_completions))
            .on_action(window.listener_for(entity, InputBaseState::on_action_go_to_definition))
    }
}

impl EditorState {
    /// The LSP providers and their cached results.
    ///
    /// This exists on the editor alone: an ordinary input or textarea has no
    /// language server, and no field to reach one through.
    pub fn lsp(&self) -> &super::Lsp {
        &self.extras.lsp
    }

    /// The LSP providers, mutably. Configure the providers through this.
    pub fn lsp_mut(&mut self) -> &mut super::Lsp {
        &mut self.extras.lsp
    }

    /// Syntax context at `offset`, or `Code` when the language has no provider.
    pub(crate) fn syntax_context_at(&self, offset: usize) -> super::SyntaxContext {
        self.mode.syntax_context_at(&self.text, offset)
    }
}

/// An unstyled source-code editor.
#[derive(IntoElement)]
pub struct Editor {
    state: Entity<EditorState>,
}

impl Editor {
    pub fn new(state: &Entity<EditorState>) -> Self {
        Self {
            state: state.clone(),
        }
    }
}

impl RenderOnce for Editor {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.state
    }
}

/// What a code editor exposes to the renderer. See [`crate::input::InputExtras`].
impl crate::input::InputExtras for super::EditorExtras {
    fn decoration_layers(&self) -> Vec<&[super::TextDecoration]> {
        self.decorations.iter().collect()
    }

    fn range_decorations(&self, ranges: &[std::ops::Range<usize>]) -> Vec<&super::RangeDecoration> {
        self.range_decorations.intersecting(ranges)
    }

    fn semantic_token_styles(
        &self,
        text: &ropey::Rope,
        range: &std::ops::Range<usize>,
        resolver: &dyn crate::input::HighlightStyleResolver,
    ) -> Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> {
        self.lsp.semantic_tokens_for_range(text, range, resolver)
    }

    fn document_color_swatches(
        &self,
        text: &ropey::Rope,
        range: &std::ops::Range<usize>,
    ) -> Vec<(std::ops::Range<usize>, gpui::Hsla)> {
        self.lsp.document_colors_for_range(text, range)
    }

    fn hover_symbol_range(&self) -> Option<std::ops::Range<usize>> {
        self.hover_popover
            .as_ref()
            .map(|session| session.symbol_range.clone())
    }

    fn inline_completion_item(&self) -> Option<&lsp_types::InlineCompletionItem> {
        self.inline_completion.item.as_ref()
    }

    fn context_menu_capabilities(&self) -> (bool, bool) {
        (
            self.lsp.definition_provider.is_some(),
            !self.lsp.code_action_providers.is_empty(),
        )
    }
}

impl InputBaseState<EditorMode> {
    /// Size and place whole lines from their text, e.g. to enlarge Markdown headings or indent
    /// and justify paragraphs.
    ///
    /// `line_style` gets each buffer line and its neighbors without their trailing `\n`
    /// ([`super::LineContext`]) and returns its [`super::LineStyle`]. It is called again for
    /// every edited line and the lines next to it, so it should be cheap and depend only on
    /// that context. Pass `None` to make every line uniform again.
    pub fn set_line_style(
        &mut self,
        line_style: Option<impl Fn(super::LineContext<'_>) -> super::LineStyle + 'static>,
        cx: &mut gpui::Context<Self>,
    ) {
        let line_style =
            line_style.map(|line_style| std::rc::Rc::new(line_style) as super::LineStyleFn);
        self.display_map.set_line_style(line_style, cx);
        cx.notify();
    }

    /// Style the buffer lines in `rows` again with the current line style, e.g. when state the
    /// style reads (through [`super::LineContext::row`]) changed without an edit on those lines.
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
            Some(std::rc::Rc::new(measure) as super::LineBlockMeasureFn),
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
    /// The block sits above the line (the first line's [`super::LineStyle::block_before`])
    /// rather than below it.
    pub above: bool,
}

/// Draws the block below a line (see [`EditorState::set_line_blocks`]).
pub type LineBlockRenderer =
    std::rc::Rc<dyn Fn(&LineBlockContext, &mut Window, &mut App) -> gpui::AnyElement>;
