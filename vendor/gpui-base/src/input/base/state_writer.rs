//! Writer patches to the input state (see `docs/patches/`): typing and clipboard transforms, and
//! revealing the collapsed lines a selection touches.

use super::*;

impl<M: InputModeKind> InputBaseState<M> {
    /// Configure a transform for committed typing and Enter, before history
    /// records the edit. Ranges are UTF-8 bytes in the pre-edit document.
    /// Programmatic edits, paste and undo/redo bypass this transform.
    pub fn set_typing_transform(
        &mut self,
        transform: impl Fn(&str, Range<usize>, &str) -> (Range<usize>, String) + 'static,
    ) {
        self.typing_transform = Some(Box::new(transform));
    }

    pub fn clear_typing_transform(&mut self) {
        self.typing_transform = None;
    }

    pub fn set_paste_transform(&mut self, transform: impl Fn(&str, bool) -> String + 'static) {
        self.paste_transform = Some(Box::new(transform));
    }

    pub(super) fn transform_typed_edit(
        &self,
        range: Range<usize>,
        text: &str,
    ) -> (Range<usize>, String) {
        match &self.typing_transform {
            Some(transform) if !self.replaying_history => {
                let edit = transform(&self.text.to_string(), range.clone(), text);
                // A suffix replacement must not consume another cursor's
                // selection. Keep the original edit when that would happen.
                if self.selections.iter().any(|selection| {
                    let other = selection.start..selection.end;
                    other != range && other.start < edit.0.end && other.end >= edit.0.start
                }) {
                    (range, text.to_string())
                } else {
                    edit
                }
            }
            _ => (range, text.to_string()),
        }
    }

    /// Transform clipboard text before insertion. `same_format` is true when
    /// Copy or Cut tagged the clipboard with this input's format identifier.
    /// Unconfigured inputs retain their usual clipboard behavior.
    pub fn clipboard_text_transform(
        mut self,
        format: impl Into<SharedString>,
        transform: impl Fn(&str, bool) -> String + 'static,
    ) -> Self {
        self.clipboard_text_format = Some(format.into());
        self.paste_transform = Some(Box::new(transform));
        self
    }

    /// Show the text of collapsed lines that a selection touches and hide the others (see
    /// [`crate::input::LineStyle::collapsed`]). Runs before each layout; does nothing unless the
    /// selected rows or the text changed.
    pub(in crate::input) fn sync_revealed_lines(&mut self, cx: &mut App) {
        if !self.display_map.has_styled_lines() {
            return;
        }
        let rows: Vec<std::ops::Range<usize>> = self
            .selections
            .iter()
            .map(|selection| {
                let start = self
                    .text
                    .offset_to_point(selection.start.min(self.text.len()))
                    .row;
                let end = self
                    .text
                    .offset_to_point(selection.end.min(self.text.len()))
                    .row;
                start..end + 1
            })
            .collect();
        let key = (self.document_revision, rows);
        if self.revealed_rows.as_ref() == Some(&key) {
            return;
        }
        self.display_map.set_revealed_rows(key.1.clone(), cx);
        self.revealed_rows = Some(key);
    }

    pub(super) fn text_clipboard_item(&self, text: String) -> ClipboardItem {
        match &self.clipboard_text_format {
            Some(format) => ClipboardItem::new_string_with_metadata(text, format.to_string()),
            None => ClipboardItem::new_string(text),
        }
    }
}
