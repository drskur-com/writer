//! Document annotations for atomic inline objects. Coordinates are always source bytes.
use std::ops::Range;

use gpui::{Context, EntityInputHandler as _, SharedString, Window};
use unicode_segmentation::UnicodeSegmentation as _;

use super::{InputBaseState, InputModeKind, undo_manager::EditIntent};

/// An application-defined reference rendered as one inline editing unit. The
/// ID names the referenced resource; the same ID may occur more than once.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InlineToken {
    id: SharedString,
    text: SharedString,
    label: SharedString,
}

impl InlineToken {
    pub fn new(id: impl Into<SharedString>, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        Self {
            id: id.into(),
            label: text.clone(),
            text,
        }
    }
    pub fn with_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }
    pub fn id(&self) -> &SharedString {
        &self.id
    }
    pub fn text(&self) -> &SharedString {
        &self.text
    }
    pub fn label(&self) -> &SharedString {
        &self.label
    }

    fn validate(&self) -> Result<(), InlineTokenError> {
        if self.id.trim().is_empty()
            || [&self.text, &self.label].iter().any(|s| {
                s.is_empty()
                    || s.chars()
                        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
            })
        {
            return Err(InlineTokenError::InvalidToken);
        }
        Ok(())
    }
}

/// A token and its current half-open UTF-8 byte range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineTokenSpan {
    range: Range<usize>,
    token: InlineToken,
}
impl InlineTokenSpan {
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }
    pub fn token(&self) -> &InlineToken {
        &self.token
    }
    fn shifted(&self, delta: isize) -> Self {
        Self {
            range: self
                .range
                .start
                .checked_add_signed(delta)
                .expect("valid token start")
                ..self
                    .range
                    .end
                    .checked_add_signed(delta)
                    .expect("valid token end"),
            token: self.token.clone(),
        }
    }
}

/// An owned, coherent text-and-token snapshot: what `content()` returns and
/// what `set_value` accepts. Plain text converts into content without tokens,
/// and every token is validated against the text as it is attached, so a
/// content value is always consistent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputContent {
    text: SharedString,
    tokens: Vec<InlineTokenSpan>,
}
impl InputContent {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            tokens: vec![],
        }
    }
    /// Attach a token to a half-open UTF-8 byte range of the text. The range
    /// must be nonempty, sit on grapheme boundaries, contain exactly the
    /// token's text and not overlap another token.
    pub fn with_token(
        mut self,
        range: Range<usize>,
        token: InlineToken,
    ) -> Result<Self, InlineTokenError> {
        token.validate()?;
        validate_range(&self.text, &range)?;
        if range.is_empty() {
            return Err(InlineTokenError::InvalidRange);
        }
        if &self.text[range.clone()] != token.text.as_ref() {
            return Err(InlineTokenError::TextMismatch);
        }
        let ix = self
            .tokens
            .partition_point(|span| span.range.end <= range.start);
        if self
            .tokens
            .get(ix)
            .is_some_and(|span| span.range.start < range.end)
        {
            return Err(InlineTokenError::OverlappingTokens);
        }
        self.tokens.insert(ix, InlineTokenSpan { range, token });
        Ok(self)
    }
    pub fn text(&self) -> &SharedString {
        &self.text
    }
    pub fn tokens(&self) -> &[InlineTokenSpan] {
        &self.tokens
    }
}

macro_rules! content_from_text {
    ($($text:ty),* $(,)?) => {
        $(impl From<$text> for InputContent {
            fn from(text: $text) -> Self {
                Self::new(text)
            }
        })*
    };
}
// Every text type `set_value` accepted before it took content.
content_from_text!(
    &str,
    &mut str,
    &String,
    String,
    char,
    Box<str>,
    std::sync::Arc<str>,
    &std::sync::Arc<str>,
    std::borrow::Cow<'_, str>,
    &SharedString,
    SharedString,
);

/// A rejected token operation never partially changes the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InlineTokenError {
    InvalidRange,
    InvalidBoundary,
    InvalidToken,
    OverlappingTokens,
    TextMismatch,
    UnsupportedMode,
    ValidationRejected,
    CompositionActive,
}

impl std::error::Error for InlineTokenError {}
impl std::fmt::Display for InlineTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidRange => "token range is outside the document or empty",
            Self::InvalidBoundary => "token range splits a Unicode grapheme",
            Self::InvalidToken => {
                "token requires a nonempty ID, text and label without control characters"
            }
            Self::OverlappingTokens => "token ranges overlap",
            Self::TextMismatch => "token text does not match its range or input normalization",
            Self::UnsupportedMode => "tokens are not supported by this input mode",
            Self::ValidationRejected => "input validation rejected the content",
            Self::CompositionActive => "finish the active IME composition before editing tokens",
        })
    }
}

fn validate_range(text: &str, range: &Range<usize>) -> Result<(), InlineTokenError> {
    if range.start > range.end || range.end > text.len() {
        return Err(InlineTokenError::InvalidRange);
    }
    let boundary =
        |offset| offset == text.len() || text.grapheme_indices(true).any(|(ix, _)| ix == offset);
    if !boundary(range.start) || !boundary(range.end) {
        return Err(InlineTokenError::InvalidBoundary);
    }
    Ok(())
}

#[derive(Default)]
pub(super) struct InlineTokenStore {
    spans: Vec<InlineTokenSpan>,
}

/// Only affected records are retained in history. Ranges are relative to the edit start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TokenDelta {
    removed: Vec<InlineTokenSpan>,
    inserted: Vec<InlineTokenSpan>,
}

impl InlineTokenStore {
    fn replace(
        &mut self,
        range: &Range<usize>,
        new_len: usize,
        inserted: &[InlineTokenSpan],
    ) -> Option<Box<TokenDelta>> {
        let mut removed = Vec::new();
        let shift = new_len as isize - range.len() as isize;
        self.spans.retain_mut(|span| {
            if span.range.start < range.end && range.start < span.range.end {
                removed.push(span.shifted(-(range.start as isize)));
                false
            } else {
                if span.range.start >= range.end {
                    *span = span.shifted(shift);
                }
                true
            }
        });
        self.spans
            .extend(inserted.iter().map(|s| s.shifted(range.start as isize)));
        if !inserted.is_empty() {
            self.spans.sort_by_key(|s| s.range.start);
        }
        (!removed.is_empty() || !inserted.is_empty()).then(|| {
            Box::new(TokenDelta {
                removed,
                inserted: inserted.to_vec(),
            })
        })
    }
}

/// Shift derived spans (Writer patch: `EditorState::set_inline_objects`) over an edit: drop the ones
/// it touches and move the ones after it. The provider derives them again before the next layout.
fn shift_derived(spans: &mut Vec<InlineTokenSpan>, range: &Range<usize>, new_len: usize) {
    let shift = new_len as isize - range.len() as isize;
    spans.retain_mut(|span| {
        if span.range.start < range.end && range.start < span.range.end {
            return false;
        }
        if span.range.start >= range.end {
            *span = span.shifted(shift);
        }
        true
    });
}

impl<M: InputModeKind> InputBaseState<M> {
    pub(super) fn token_spans(&self) -> &[InlineTokenSpan] {
        self.inline_tokens
            .as_ref()
            .map_or(&self.derived_tokens, |store| &store.spans)
    }
    /// The height an object starting at `start` gets: the row height of its line (Writer patch),
    /// so objects grow with taller rows (a heading, or a line style that makes room for them).
    pub(super) fn object_line_height(&self, start: usize, line_height: gpui::Pixels) -> gpui::Pixels {
        use super::rope_ext::RopeExt as _;
        if !M::CODE_EDITOR {
            return line_height;
        }
        let row = self.text.offset_to_point(start.min(self.text.len())).row;
        let scale = self
            .display_map
            .line(row)
            .map_or(1., |line| line.style.height_scale);
        line_height * scale.max(0.1)
    }

    /// Derive the inline objects from the text again if it or the selections changed since the
    /// last time (Writer patch). Ranges that split a character, overlap, span lines, or touch an
    /// IME composition are skipped. An object a caret touches (its ends included) or a selection
    /// cuts into is shown as its source text, followed by its element as an empty-range decoration, so the caret can
    /// enter it and edit the source.
    pub(crate) fn refresh_derived_tokens(&mut self) {
        use super::rope_ext::RopeExt as _;
        let Some(provider) = self.inline_object_ranges.clone() else {
            if !self.derived_tokens.is_empty() {
                self.derived_tokens.clear();
            }
            return;
        };
        let selections: Vec<(usize, usize)> = self
            .selections
            .iter()
            .map(|selection| (selection.start.min(selection.end), selection.start.max(selection.end)))
            .collect();
        let key = (self.document_revision, selections);
        if self.derived_revision.as_ref() == Some(&key) {
            return;
        }
        let mut spans = Vec::new();
        for row in 0..self.text.lines_len() {
            let line_start = self.text.line_start_offset(row);
            let line: String = self.text.slice_line(row).into();
            let mut end = 0;
            for (range, token) in provider(row, &line) {
                let valid = range.start >= end
                    && range.start < range.end
                    && range.end <= line.len()
                    && line.is_char_boundary(range.start)
                    && line.is_char_boundary(range.end)
                    && token.validate().is_ok();
                if !valid {
                    continue;
                }
                let range = line_start + range.start..line_start + range.end;
                if self
                    .ime_marked_range
                    .as_ref()
                    .is_some_and(|marked| marked.start < range.end && range.start < marked.end)
                {
                    continue;
                }
                end = range.end - line_start;
                spans.push(InlineTokenSpan { range, token });
            }
        }
        // A caret between two adjacent objects opens only one: the one it moved into (the right
        // one when it moved forward without an edit), otherwise the one before it.
        let previous = self
            .derived_revision
            .as_ref()
            .filter(|(revision, _)| *revision == key.0)
            .map(|(_, selections)| selections.as_slice())
            .unwrap_or_default();
        let mut open = vec![false; spans.len()];
        for (ix, &(start, end)) in key.1.iter().enumerate() {
            let first = spans.partition_point(|span| span.range.end < start);
            let touched = (first..spans.len()).take_while(|&i| spans[i].range.start <= end);
            if start < end {
                // A selection opens only the objects it cuts into; one it covers whole stays an
                // object, shown selected.
                touched
                    .filter(|&i| {
                        let range = &spans[i].range;
                        (range.start < start && start < range.end)
                            || (range.start < end && end < range.end)
                    })
                    .for_each(|i| open[i] = true);
                continue;
            }
            let forward = previous
                .get(ix)
                .is_some_and(|&(before, after)| before == after && before < start);
            let touched: Vec<usize> = touched.collect();
            let pick = match touched.as_slice() {
                [_, right] if forward => *right,
                [left, ..] => *left,
                [] => continue,
            };
            open[pick] = true;
        }
        for (span, open) in spans.iter_mut().zip(open) {
            if open {
                span.range = span.range.end..span.range.end;
            }
        }
        if spans != self.derived_tokens {
            self.derived_tokens = spans;
            // Which objects are shown changed: measure and wrap them again.
            self.token_layout_cache = None;
        }
        self.derived_revision = Some(key);
    }
    pub(super) fn token_boundary(&self, offset: usize, bias: sum_tree::Bias) -> usize {
        let spans = self.token_spans();
        let ix = spans.partition_point(|s| s.range.end <= offset);
        if let Some(span) = spans.get(ix).filter(|s| s.range.start < offset) {
            if bias == sum_tree::Bias::Left {
                span.range.start
            } else {
                span.range.end
            }
        } else {
            offset
        }
    }
    pub(super) fn normalize_token_range(&self, range: Range<usize>) -> Range<usize> {
        if self.replaying_history {
            return range;
        }
        if range.is_empty() {
            let offset = self.token_boundary(range.start, sum_tree::Bias::Right);
            offset..offset
        } else {
            self.token_boundary(range.start, sum_tree::Bias::Left)
                ..self.token_boundary(range.end, sum_tree::Bias::Right)
        }
    }
    pub(super) fn edit_tokens(
        &mut self,
        range: &Range<usize>,
        new_len: usize,
    ) -> Option<Box<TokenDelta>> {
        if self.inline_tokens.is_none() && !self.derived_tokens.is_empty() {
            shift_derived(&mut self.derived_tokens, range, new_len);
        }
        if self.replaying_history {
            return None;
        }
        let inserted = self.pending_token.take().map(|token| InlineTokenSpan {
            range: 0..new_len,
            token,
        });
        if inserted.is_some() && self.inline_tokens.is_none() {
            self.inline_tokens = Some(Box::default());
        }
        self.inline_tokens
            .as_mut()?
            .replace(range, new_len, inserted.as_slice())
    }
    pub(super) fn replay_tokens(
        &mut self,
        range: &Range<usize>,
        new_len: usize,
        delta: Option<&TokenDelta>,
        undo: bool,
    ) {
        let inserted = delta
            .map(|d| {
                if undo {
                    &d.removed[..]
                } else {
                    &d.inserted[..]
                }
            })
            .unwrap_or_default();
        if !inserted.is_empty() && self.inline_tokens.is_none() {
            self.inline_tokens = Some(Box::default());
        }
        if let Some(store) = self.inline_tokens.as_mut() {
            store.replace(range, new_len, inserted);
        }
    }
    fn check_token_mode(&self) -> Result<(), InlineTokenError> {
        if self.ime_marked_range.is_some() {
            return Err(InlineTokenError::CompositionActive);
        }
        if M::CODE_EDITOR || self.masked || self.token_is_secret() || !self.mask_pattern.is_none() {
            return Err(InlineTokenError::UnsupportedMode);
        }
        Ok(())
    }
    fn replace_token(
        &mut self,
        range: Range<usize>,
        token: InlineToken,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), InlineTokenError> {
        self.check_token_mode()?;
        token.validate()?;
        let text = self.text.to_string();
        validate_range(&text, &range)?;
        let range = self.normalize_token_range(range);
        let mut next = text;
        next.replace_range(range.clone(), &token.text);
        validate_range(&next, &(range.start..range.start + token.text.len()))?;
        if self.normalize_input(&next) != next {
            return Err(InlineTokenError::TextMismatch);
        }
        if !self.is_valid_input(&next, cx) {
            return Err(InlineTokenError::ValidationRejected);
        }
        if self
            .token_spans()
            .iter()
            .any(|s| s.range == range && s.token == token)
        {
            return Ok(());
        }
        let new_text = token.text.clone();
        self.pending_token = Some(token);
        self.undo_manager.break_transaction_coalescing();
        self.undo_manager.set_pending_intent(EditIntent::Atomic);
        let range_utf16 = self.range_to_utf16(&range);
        self.validated_token_edit = true;
        self.with_edits_allowed(|state| {
            state.replace_text_in_range(Some(range_utf16), &new_text, window, cx)
        });
        self.validated_token_edit = false;
        self.pending_token = None;
        Ok(())
    }
    /// Adopt the tokens of content whose text was just installed by
    /// `set_value`. Tokens are dropped when this mode cannot show them or when
    /// normalization changed the text, since their ranges would no longer
    /// describe it.
    pub(super) fn install_tokens(&mut self, content: InputContent) {
        let supported = !M::CODE_EDITOR && self.mask_pattern.is_none();
        let text_kept = self.text == content.text.as_ref();
        self.inline_tokens = (supported && text_kept && !content.tokens.is_empty()).then(|| {
            Box::new(InlineTokenStore {
                spans: content.tokens,
            })
        });
    }
}

macro_rules! token_api {
    ($mode:ty) => {
        impl InputBaseState<$mode> {
            /// Replace the selection with an atomic token; no delimiter is added.
            pub fn replace_with_token(
                &mut self,
                token: InlineToken,
                window: &mut Window,
                cx: &mut Context<Self>,
            ) -> Result<(), InlineTokenError> {
                self.replace_token(self.selected_range(), token, window, cx)
            }
            /// Replace a UTF-8 byte range, expanding overlaps to whole tokens.
            pub fn replace_range_with_token(
                &mut self,
                range: Range<usize>,
                token: InlineToken,
                window: &mut Window,
                cx: &mut Context<Self>,
            ) -> Result<(), InlineTokenError> {
                self.replace_token(range, token, window, cx)
            }
            pub fn tokens(&self) -> &[InlineTokenSpan] {
                self.token_spans()
            }
            /// The text with its tokens, as `set_value` accepts it.
            pub fn content(&self) -> InputContent {
                InputContent {
                    text: self.value(),
                    tokens: self.token_spans().to_vec(),
                }
            }
        }
    };
}
token_api!(super::InputMode);
token_api!(super::TextareaMode);
