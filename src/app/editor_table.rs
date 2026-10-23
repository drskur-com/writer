//! 에디터의 표를 격자로 그린다(ADR-0017).
//!
//! 표 행(`| … |`)은 그대로 고칠 수 있는 글자로 두고, 마지막 행 아래에 gpui-base 줄 블록
//! (`EditorState::set_line_blocks`)으로 격자를 그린다. 표의 줄 글자는 커서가 표에 닿을 때만 모두
//! 보인다(`LineStyle::collapsed`·`reveal_run`). 고치는 동안에도 격자는 아래에 남아 결과를 보여 준다.
//!
//! 블록 높이는 그리기 전에 정해야 해서 칸 글자를 gpui 줄바꿈(`LineWrapper`)으로 미리 나누고, 나눈 줄을
//! 그대로 그린다. 그래서 칸 글자는 꾸밈 없이 칸마다 한 서체(머리 행은 굵게)로 그린다.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::gpui::prelude::FluentBuilder as _;
use gpui_kit::gpui::*;
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use super::book_preview::column_widths;
use super::style::Palette;
use crate::markdown::{self, ColumnAlign, Table};

/// 칸 글자 크기(본문 대비).
const TEXT_SCALE: f32 = 0.9;

/// 칸 글자 줄 높이(글자 크기 대비).
const LINE_HEIGHT: f32 = 1.5;

/// 칸 좌우·위아래 여백.
const CELL_PADDING_X: f32 = 13.0;
const CELL_PADDING_Y: f32 = 6.0;

/// 격자 위·아래 여백(이미지 블록과 같다).
const PADDING_TOP: f32 = 4.0;
const PADDING_BOTTOM: f32 = 16.0;

/// 본문의 표 하나.
struct EditorTable {
    /// 에디터 줄 범위.
    rows: Range<usize>,
    /// 표 원문(줄 범위의 글자). 바뀌었는지 비교할 때 쓴다.
    source: String,
    table: Table,
    /// 마지막으로 잰 격자와 그 폭.
    grid: RefCell<Option<(Pixels, Rc<Grid>)>>,
}

/// 잰 격자. 위치는 블록 왼쪽 위 기준이다.
struct Grid {
    height: Pixels,
    /// 열마다 (왼쪽, 폭, 정렬). 폭은 칸 여백을 뺀 글자 폭이다.
    columns: Vec<(Pixels, Pixels, ColumnAlign)>,
    rows: Vec<GridRow>,
}

struct GridRow {
    top: Pixels,
    height: Pixels,
    /// 칸마다 줄바꿈한 줄.
    cells: Vec<Vec<SharedString>>,
}

/// 열린 시트의 표 목록과 격자 배치. 줄 스타일·줄 블록의 측정·그리기에서 함께 쓴다.
pub(super) struct TableBlocks {
    text_system: Arc<TextSystem>,
    font_size: Cell<Pixels>,
    family: RefCell<SharedString>,
    /// 줄 순서대로.
    tables: RefCell<Vec<Rc<EditorTable>>>,
}

impl TableBlocks {
    pub(super) fn new(
        text: &str,
        family: SharedString,
        font_size: Pixels,
        text_system: Arc<TextSystem>,
    ) -> Self {
        let blocks = Self {
            text_system,
            font_size: Cell::new(font_size * TEXT_SCALE),
            family: RefCell::new(family),
            tables: RefCell::default(),
        };
        blocks.sync(text);
        blocks
    }

    /// 본문에 맞춰 표를 다시 찾는다. 바뀐 표(위치·글자)가 닿았던 줄과 닿는 줄을 돌려준다.
    /// 편집한 줄은 엔진이 맞추기 전 목록으로 배치했을 수 있어서, 이 줄들을 다시 배치해야 한다.
    pub(super) fn sync(&self, text: &str) -> Vec<Range<usize>> {
        let lines: Vec<&str> = text.split('\n').collect();
        let old = self.tables.take();
        let mut new = Vec::new();
        for rows in crate::text_transform::table_runs(text) {
            let source = lines[rows.clone()].join("\n");
            let kept = old
                .iter()
                .find(|table| table.rows == rows && table.source == source);
            if let Some(table) = kept {
                new.push(table.clone());
            } else if let Some(table) = markdown::table(&source) {
                new.push(Rc::new(EditorTable {
                    rows,
                    source,
                    table,
                    grid: RefCell::default(),
                }));
            }
        }
        let changed = |from: &[Rc<EditorTable>], to: &[Rc<EditorTable>]| {
            from.iter()
                .filter(|table| !to.iter().any(|other| Rc::ptr_eq(table, other)))
                .map(|table| table.rows.clone())
                .collect::<Vec<_>>()
        };
        let mut rows = changed(&old, &new);
        rows.extend(changed(&new, &old));
        self.tables.replace(new);
        rows
    }

    /// 서체를 바꾼다. 바뀌었으면 격자를 다시 재야 한다(`true`).
    pub(super) fn set_family(&self, family: &SharedString) -> bool {
        if *self.family.borrow() == *family {
            return false;
        }
        self.family.replace(family.clone());
        for table in self.tables.borrow().iter() {
            table.grid.take();
        }
        true
    }

    pub(super) fn set_font_size(&self, size: Pixels) {
        let size = size * TEXT_SCALE;
        if self.font_size.replace(size) != size {
            for table in self.tables.borrow().iter() {
                table.grid.take();
            }
        }
    }

    /// 줄이 표 행인지.
    pub(super) fn contains(&self, row: usize) -> bool {
        self.table_at(row).is_some()
    }

    /// 줄이 표의 첫 행인지.
    pub(super) fn starts(&self, row: usize) -> bool {
        self.table_at(row)
            .is_some_and(|table| table.rows.start == row)
    }

    /// 줄 아래 격자 높이. 표의 마지막 행이 아니면 0이다.
    pub(super) fn measure(&self, row: usize, wrap_width: Option<Pixels>) -> Pixels {
        let Some(table) = self.table_at(row).filter(|table| table.rows.end == row + 1) else {
            return px(0.);
        };
        let width = wrap_width.unwrap_or(px(super::editor::EDITOR_MAX_WIDTH));
        self.grid(&table, width).height
    }

    /// 표 마지막 행 아래에 격자를 그린다. 표가 아니면 `None`이다.
    pub(super) fn render(&self, row: usize, size: Size<Pixels>, cx: &App) -> Option<AnyElement> {
        let table = self
            .table_at(row)
            .filter(|table| table.rows.end == row + 1)?;
        let grid = self.grid(&table, size.width);
        let pal = Palette::from_theme(cx.theme());
        let line_height = self.font_size.get() * LINE_HEIGHT;
        let rule = |top: Pixels| {
            div()
                .absolute()
                .left_0()
                .top(top)
                .w(size.width)
                .h(px(1.))
                .bg(pal.border)
        };
        let mut root = div()
            .relative()
            .w(size.width)
            .h(grid.height)
            .font_family(self.family.borrow().clone())
            .text_size(self.font_size.get())
            .line_height(line_height)
            .text_color(pal.foreground);
        // 행 배경을 먼저 그려 테두리가 배경에 가려지지 않게 한다.
        for (row_ix, row) in grid.rows.iter().enumerate() {
            root = root.child(
                div()
                    .absolute()
                    .left_0()
                    .top(row.top)
                    .w(size.width)
                    .h(row.height)
                    .bg(if row_ix % 2 == 1 {
                        pal.tint
                    } else {
                        pal.background
                    }),
            );
        }
        if let Some(first) = grid.rows.first() {
            root = root.child(rule(first.top));
        }
        for (row_ix, row) in grid.rows.iter().enumerate() {
            let header = row_ix == 0;
            for (lines, &(left, width, align)) in row.cells.iter().zip(&grid.columns) {
                let cell = div()
                    .absolute()
                    .left(left)
                    .top(row.top + px(CELL_PADDING_Y))
                    .w(width)
                    .overflow_hidden()
                    .when(header, |this| this.font_weight(FontWeight::BOLD))
                    .children(lines.iter().map(|line| {
                        div()
                            .flex()
                            .w(width)
                            .h(line_height)
                            .whitespace_nowrap()
                            .map(|this| match align {
                                ColumnAlign::Left => this.justify_start(),
                                ColumnAlign::Center => this.justify_center(),
                                ColumnAlign::Right => this.justify_end(),
                            })
                            .child(line.clone())
                    }));
                root = root.child(cell);
            }
            root = root.child(rule(row.top + row.height));
        }
        if let (Some(first), Some(last)) = (grid.rows.first(), grid.rows.last()) {
            let height = last.top + last.height - first.top + px(1.);
            let vertical_rule = |left: Pixels| {
                div()
                    .absolute()
                    .left(left)
                    .top(first.top)
                    .w(px(1.))
                    .h(height)
                    .bg(pal.border)
            };
            root = root.child(vertical_rule(px(0.)));
            for &(left, _, _) in grid.columns.iter().skip(1) {
                root = root.child(vertical_rule(left - px(CELL_PADDING_X)));
            }
            root = root.child(vertical_rule((size.width - px(1.)).max(px(0.))));
        }
        Some(root.into_any_element())
    }

    fn table_at(&self, row: usize) -> Option<Rc<EditorTable>> {
        let tables = self.tables.borrow();
        let ix = tables.partition_point(|table| table.rows.end <= row);
        tables
            .get(ix)
            .filter(|table| table.rows.contains(&row))
            .cloned()
    }

    /// 폭 `width`에 맞춘 격자. 같은 폭이면 지난번 것을 쓴다.
    fn grid(&self, table: &EditorTable, width: Pixels) -> Rc<Grid> {
        if let Some((cached, grid)) = &*table.grid.borrow()
            && *cached == width
        {
            return grid.clone();
        }
        let grid = Rc::new(self.layout(&table.table, width));
        table.grid.replace(Some((width, grid.clone())));
        grid
    }

    fn layout(&self, table: &Table, width: Pixels) -> Grid {
        let columns = table.columns();
        let regular = font(self.family.borrow().clone());
        let bold = Font {
            weight: FontWeight::BOLD,
            ..regular.clone()
        };
        let texts: Vec<Vec<String>> = table
            .rows
            .iter()
            .map(|row| {
                (0..columns)
                    .map(|column| row.get(column).map(|cell| markdown::rich_text(cell)))
                    .map(|text| text.unwrap_or_default().trim().to_string())
                    .collect()
            })
            .collect();
        // 열 너비는 칸 글자를 한 줄로 놓았을 때의 폭을 따른다(미리보기와 같은 규칙).
        let advance = |font_id: FontId, text: &str| -> f32 {
            text.chars()
                .map(|c| {
                    self.text_system
                        .advance(font_id, self.font_size.get(), c)
                        .map_or(0., |size| f32::from(size.width))
                })
                .sum()
        };
        let (regular_id, bold_id) = (
            self.text_system.resolve_font(&regular),
            self.text_system.resolve_font(&bold),
        );
        let mut natural = vec![0f32; columns];
        for (row_ix, row) in texts.iter().enumerate() {
            let font_id = if row_ix == 0 { bold_id } else { regular_id };
            for (column, text) in row.iter().enumerate() {
                for segment in text.split('\n') {
                    natural[column] = natural[column].max(advance(font_id, segment));
                }
            }
        }
        let padding = px(CELL_PADDING_X) * 2.;
        let available = f32::from(width - padding * columns as f32).max(0.);
        let mut left = px(0.);
        let columns: Vec<(Pixels, Pixels, ColumnAlign)> = column_widths(&natural, available)
            .into_iter()
            .enumerate()
            .map(|(column, inner)| {
                let entry = (left + px(CELL_PADDING_X), px(inner), table.align(column));
                left += px(inner) + padding;
                entry
            })
            .collect();
        let line_height = self.font_size.get() * LINE_HEIGHT;
        let mut top = px(PADDING_TOP);
        let rows = texts
            .iter()
            .enumerate()
            .map(|(row_ix, row)| {
                let mut wrapper = self.text_system.line_wrapper(
                    if row_ix == 0 {
                        bold.clone()
                    } else {
                        regular.clone()
                    },
                    self.font_size.get(),
                );
                let cells: Vec<Vec<SharedString>> = row
                    .iter()
                    .zip(&columns)
                    .map(|(text, &(_, width, _))| wrap(&mut wrapper, text, width))
                    .collect();
                let lines = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
                let height = line_height * lines as f32 + px(CELL_PADDING_Y) * 2.;
                let row = GridRow { top, height, cells };
                top += height;
                row
            })
            .collect();
        Grid {
            height: top + px(PADDING_BOTTOM),
            columns,
            rows,
        }
    }
}

/// 칸 글자를 폭에 맞춰 줄로 나눈다. `\n`은 강제 줄바꿈이다.
fn wrap(wrapper: &mut LineWrapper, text: &str, width: Pixels) -> Vec<SharedString> {
    let mut lines = Vec::new();
    for segment in text.split('\n') {
        let mut start = 0;
        let fragments = [LineFragment::text(segment)];
        for boundary in wrapper.wrap_line(&fragments, width, IndentAdjustment::NoIndent) {
            lines.push(segment[start..boundary.ix].trim_end().to_string().into());
            start = boundary.ix;
        }
        lines.push(segment[start..].trim_end().to_string().into());
    }
    lines
}

#[cfg(test)]
mod tests {
    // `gpui::*`의 `test` 매크로와 겹치지 않도록 하나씩 가져온다.
    use super::{CELL_PADDING_Y, LINE_HEIGHT, PADDING_BOTTOM, PADDING_TOP, TableBlocks};
    use gpui_kit::gpui::{NoopTextSystem, TextSystem, px};
    use std::sync::Arc;

    /// 글자마다 폭이 같은 가짜 서체(글자 크기의 0.6배).
    fn blocks(text: &str) -> TableBlocks {
        let text_system = Arc::new(TextSystem::new(Arc::new(NoopTextSystem::new())));
        TableBlocks::new(text, "Test".into(), px(10.), text_system)
    }

    #[test]
    fn grid_hangs_below_the_last_row() {
        let blocks = blocks("앞\n| a | b |\n|---|--:|\n| 1 | 2 |\n뒤");
        assert!(!blocks.contains(0));
        assert!((1..4).all(|row| blocks.contains(row)));
        assert!(!blocks.contains(4));
        assert_eq!(blocks.measure(2, Some(px(400.))), px(0.));
        // 위아래 여백 + 두 행(한 줄 + 칸 여백).
        let row = px(9. * LINE_HEIGHT + CELL_PADDING_Y * 2.);
        assert_eq!(
            blocks.measure(3, Some(px(400.))),
            px(PADDING_TOP + PADDING_BOTTOM) + row * 2.
        );
    }

    #[test]
    fn narrow_columns_wrap_cells_into_taller_rows() {
        let blocks = blocks("| a |\n|---|\n| one two three four |");
        let wide = blocks.measure(2, Some(px(400.)));
        let narrow = blocks.measure(2, Some(px(60.)));
        assert!(narrow > wide, "{narrow:?} > {wide:?}");
    }

    #[test]
    fn rows_without_a_delimiter_are_not_a_table() {
        let blocks = blocks("| a |\n| b |");
        assert!(!blocks.contains(0));
    }

    #[test]
    fn sync_reports_the_rows_of_changed_tables_only() {
        let blocks = blocks("| a |\n|---|\n\n| b |\n|---|");
        // 둘째 표만 고쳤다.
        assert_eq!(blocks.sync("| a |\n|---|\n\n| c |\n|---|"), [3..5, 3..5]);
        // 첫 표 위에 줄이 생겨 두 표가 모두 밀렸다.
        assert_eq!(
            blocks.sync("새 줄\n| a |\n|---|\n\n| c |\n|---|"),
            [0..2, 3..5, 1..3, 4..6]
        );
        // 그대로면 다시 배치할 줄이 없다.
        assert!(
            blocks
                .sync("새 줄\n| a |\n|---|\n\n| c |\n|---|")
                .is_empty()
        );
        // 표가 깨지면 그 줄을 다시 배치한다(줄이 줄어 둘째 표도 당겨졌다).
        assert_eq!(
            blocks.sync("새 줄\n| a |\n\n| c |\n|---|"),
            [1..3, 4..6, 3..5]
        );
    }
}
