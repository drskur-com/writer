//! 에디터 펜스 코드 블록(```` ```js ````) 문법 하이라이팅.
//!
//! gpui-component의 tree-sitter 하이라이터로 원고를 markdown으로 파싱하면, markdown 문법의
//! 인젝션 규칙이 펜스 안쪽을 그 언어 문법으로 칠한다. 제목·링크 같은 본문 표시는
//! `editor.rs`의 마크업 장식이 맡으므로, 펜스 안쪽(`code_fence_content`) 밖의 색은 버린다.

use std::ops::Range;
use std::rc::Rc;

use gpui_base::input::{
    EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter,
    InputHighlighterFactory, Rope,
};
use gpui_kit::component::highlighter::SyntaxHighlighter;
use gpui_kit::gpui::{Context, HighlightStyle, SharedString, Window};

/// 에디터가 원고에 쓰는 언어 이름.
pub(super) const LANGUAGE: &str = "markdown";

/// `EditorState::set_highlighter_factory`에 넘길 팩토리. markdown만 맡는다.
pub(super) fn factory() -> InputHighlighterFactory {
    Rc::new(|language| {
        (language == LANGUAGE)
            .then(|| Box::new(CodeFenceHighlighter::new()) as Box<dyn InputHighlighter>)
    })
}

struct CodeFenceHighlighter {
    inner: SyntaxHighlighter,
    /// 펜스 안쪽 바이트 범위. 시작 순으로 정렬돼 있고 겹치지 않는다.
    fences: Vec<Range<usize>>,
}

impl CodeFenceHighlighter {
    fn new() -> Self {
        Self {
            inner: SyntaxHighlighter::new(LANGUAGE),
            fences: Vec::new(),
        }
    }

    fn collect_fences(&mut self) {
        self.fences.clear();
        let Some(tree) = self.inner.tree() else {
            return;
        };
        let mut cursor = tree.walk();
        loop {
            let node = cursor.node();
            if node.kind() == "code_fence_content" {
                self.fences.push(node.byte_range());
            } else if cursor.goto_first_child() {
                continue;
            }
            // 다음 형제로, 없으면 부모의 다음 형제로 올라간다.
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    return;
                }
            }
        }
    }
}

impl InputHighlighter for CodeFenceHighlighter {
    fn language(&self) -> SharedString {
        LANGUAGE.into()
    }

    fn update(
        &mut self,
        edit: Option<InputEdit>,
        text: &Rope,
        _folding: bool,
        _window: &mut Window,
        _cx: &mut Context<EditorState>,
    ) {
        // 원고 한 편은 짧아서 끝까지 바로 파싱한다.
        self.inner.update(edit.map(to_tree_sitter_edit), text, None);
        self.collect_fences();
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let runs = self.inner.styles(range, &CodeOnly(resolver));
        mask_outside(runs, &self.fences)
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

/// markdown 문법이 코드 블록 전체에 붙이는 `text.literal`을 버린다.
/// 남겨 두면 언어 문법이 이름을 붙이지 않은 글자(변수 이름 등)까지 그 색으로 칠해진다.
struct CodeOnly<'a>(&'a dyn HighlightStyleResolver);

impl HighlightStyleResolver for CodeOnly<'_> {
    fn style(&self, name: &str) -> Option<HighlightStyle> {
        (name != "text.literal")
            .then(|| self.0.style(name))
            .flatten()
    }
}

/// `fences` 밖에 걸친 스타일을 기본 스타일로 바꾼다. 줄 범위는 그대로 이어진다.
fn mask_outside(
    runs: Vec<(Range<usize>, HighlightStyle)>,
    fences: &[Range<usize>],
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut out: Vec<(Range<usize>, HighlightStyle)> = Vec::with_capacity(runs.len());
    let mut push = |range: Range<usize>, style: HighlightStyle| {
        if range.is_empty() {
            return;
        }
        match out.last_mut() {
            Some((last, last_style)) if last.end == range.start && *last_style == style => {
                last.end = range.end;
            }
            _ => out.push((range, style)),
        }
    };
    for (run, style) in runs {
        let mut pos = run.start;
        let first = fences.partition_point(|fence| fence.end <= run.start);
        for fence in fences[first..]
            .iter()
            .take_while(|fence| fence.start < run.end)
        {
            push(
                pos..fence.start.max(pos).min(run.end),
                HighlightStyle::default(),
            );
            let inside = fence.start.max(pos)..fence.end.min(run.end);
            pos = inside.end;
            push(inside, style);
        }
        push(pos..run.end, HighlightStyle::default());
    }
    out
}

fn to_tree_sitter_edit(edit: InputEdit) -> tree_sitter::InputEdit {
    let point = |p: gpui_base::input::Point| tree_sitter::Point::new(p.row, p.column);
    tree_sitter::InputEdit {
        start_byte: edit.start_byte,
        old_end_byte: edit.old_end_byte,
        new_end_byte: edit.new_end_byte,
        start_position: point(edit.start_position),
        old_end_position: point(edit.old_end_position),
        new_end_position: point(edit.new_end_position),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::gpui::FontWeight;

    fn bold() -> HighlightStyle {
        FontWeight::BOLD.into()
    }

    #[test]
    fn keeps_styles_only_inside_fences() {
        let runs = vec![
            (0..10, bold()),
            (10..20, HighlightStyle::default()),
            (20..30, bold()),
        ];
        let masked = mask_outside(runs, &[5..12, 25..40]);
        assert_eq!(
            masked,
            vec![
                (0..5, HighlightStyle::default()),
                (5..10, bold()),
                (10..25, HighlightStyle::default()),
                (25..30, bold()),
            ]
        );
    }

    #[test]
    fn finds_fence_content() {
        let text = "# 제목\n\n```js\nconst a = 1;\n```\n\n본문\n";
        let mut highlighter = CodeFenceHighlighter::new();
        highlighter.inner.update(None, &Rope::from(text), None);
        highlighter.collect_fences();
        let start = text.find("const").unwrap();
        let end = text.rfind("```").unwrap();
        assert_eq!(highlighter.fences, vec![start..end]);
    }

    #[test]
    fn colors_injected_code_only() {
        let text = "# 제목\n\n```js\nconst hello = \"world\";\n```\n";
        let mut highlighter = CodeFenceHighlighter::new();
        highlighter.inner.update(None, &Rope::from(text), None);
        highlighter.collect_fences();
        let theme = gpui_kit::component::highlighter::HighlightTheme::default_dark();
        let styles = highlighter.styles(&(0..text.len()), &*theme);
        let colored: Vec<&str> = styles
            .iter()
            .filter(|(_, style)| *style != HighlightStyle::default())
            .map(|(range, _)| &text[range.clone()])
            .collect();
        assert!(colored.contains(&"const"), "{colored:?}");
        assert!(colored.contains(&"\"world\""), "{colored:?}");
        assert!(!colored.iter().any(|s| s.contains("hello")), "{colored:?}");
        assert!(!colored.iter().any(|s| s.contains("제목")), "{colored:?}");
    }
}
