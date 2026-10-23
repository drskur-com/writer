//! A visual row whose offsets remain in source UTF-8 bytes.
use gpui::{App, Pixels, Point, ShapedLine, SharedString, TextAlign, Window, point, px};
use std::ops::Range;

pub(crate) struct InlineFragment {
    pub(crate) range: Range<usize>,
    pub(crate) x: Pixels,
    pub(crate) width: Pixels,
    pub(crate) text: Option<ShapedLine>,
}

pub(crate) struct InputLine {
    pub(crate) len: usize,
    pub(crate) width: Pixels,
    pub(crate) text: SharedString,
    content: Content,
}
// Keep the ordinary shaped row inline: boxing it would add an allocation to
// every existing plain-text row. Only token rows allocate fragment storage.
#[allow(clippy::large_enum_variant)]
enum Content {
    Text(ShapedLine),
    Inline(Vec<InlineFragment>),
}
/// The character boundary of `line` nearest to `x`.
///
/// [`ShapedLine::closest_index_for_x`] puts any `x` past the start of the last glyph at the line
/// end, which is wrong for a fragment followed by more text (e.g. a word of a justified row).
fn closest_boundary(line: &ShapedLine, x: Pixels) -> usize {
    line.text
        .char_indices()
        .map(|(ix, _)| ix)
        .chain(std::iter::once(line.len))
        .min_by(|&a, &b| {
            let distance = |ix| (line.x_for_index(ix) - x).abs();
            f32::from(distance(a)).total_cmp(&f32::from(distance(b)))
        })
        .unwrap_or(0)
}

impl From<ShapedLine> for InputLine {
    fn from(line: ShapedLine) -> Self {
        Self {
            len: line.len,
            width: line.width,
            text: line.text.clone(),
            content: Content::Text(line),
        }
    }
}
impl InputLine {
    pub(crate) fn inline(text: SharedString, fragments: Vec<InlineFragment>) -> Self {
        let width = fragments.last().map_or(px(0.), |f| f.x + f.width);
        Self {
            len: text.len(),
            text,
            width,
            content: Content::Inline(fragments),
        }
    }
    pub(crate) fn x_for_index(&self, ix: usize) -> Pixels {
        match &self.content {
            Content::Text(line) => line.x_for_index(ix),
            Content::Inline(fragments) => {
                for (at, f) in fragments.iter().enumerate() {
                    // A caret at the start of an empty-range decoration (Writer patch) stays
                    // before it, right after the text it follows.
                    let before_decoration = ix == f.range.end
                        && f.text.is_some()
                        && fragments.get(at + 1).is_some_and(|next| {
                            next.text.is_none() && next.range.is_empty()
                        });
                    if ix < f.range.end || before_decoration {
                        return f.x
                            + f.text.as_ref().map_or(
                                if ix <= f.range.start { px(0.) } else { f.width },
                                |line| line.x_for_index(ix.saturating_sub(f.range.start)),
                            );
                    }
                }
                self.width
            }
        }
    }
    pub(crate) fn closest_index_for_x(&self, x: Pixels) -> usize {
        match &self.content {
            Content::Text(line) => line.closest_index_for_x(x),
            Content::Inline(fragments) => {
                for f in fragments {
                    if x <= f.x + f.width {
                        return f.text.as_ref().map_or(
                            if x - f.x < f.width / 2. {
                                f.range.start
                            } else {
                                f.range.end
                            },
                            |line| f.range.start + closest_boundary(line, x - f.x),
                        );
                    }
                }
                self.len
            }
        }
    }
    pub(crate) fn index_for_x(&self, x: Pixels) -> Option<usize> {
        match &self.content {
            Content::Text(line) => line.index_for_x(x),
            Content::Inline(_) => {
                (x >= px(0.) && x <= self.width).then(|| self.closest_index_for_x(x))
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint(
        &self,
        pos: Point<Pixels>,
        height: Pixels,
        align: TextAlign,
        width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.paint_pass(pos, height, align, width, false, window, cx);
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn paint_background(
        &self,
        pos: Point<Pixels>,
        height: Pixels,
        align: TextAlign,
        width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.paint_pass(pos, height, align, width, true, window, cx);
    }
    #[allow(clippy::too_many_arguments)]
    fn paint_pass(
        &self,
        pos: Point<Pixels>,
        height: Pixels,
        align: TextAlign,
        width: Option<Pixels>,
        background: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let paint = |line: &ShapedLine, pos, align, width, window: &mut Window, cx: &mut App| {
            if background {
                let _ = line.paint_background(pos, height, align, width, window, cx);
            } else {
                let _ = line.paint(pos, height, align, width, window, cx);
            }
        };
        match &self.content {
            Content::Text(line) => paint(line, pos, align, width, window, cx),
            Content::Inline(fragments) => {
                let remaining = (width.unwrap_or(self.width) - self.width).max(px(0.));
                let offset = match align {
                    TextAlign::Right => remaining,
                    TextAlign::Center => remaining / 2.,
                    _ => px(0.),
                };
                for f in fragments {
                    if let Some(line) = &f.text {
                        paint(
                            line,
                            pos + point(offset + f.x, px(0.)),
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                }
            }
        }
    }
}
