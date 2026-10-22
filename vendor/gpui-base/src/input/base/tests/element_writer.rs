//! Tests for the writer patches to the text element: line styles, line blocks, collapsed lines,
//! line overlays and row placement (see `docs/patches/`).

use super::*;

#[gpui::test]
fn styled_lines_get_their_own_row_height_and_text_size(cx: &mut TestAppContext) {
    let text = "# Title\nbody\nmore";
    let (editor, window) = decoration_editor(cx, text, false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, cx| {
        editor.update(cx, |state, cx| {
            state.set_line_style(
                Some(|line: crate::input::LineContext<'_>| {
                    if line.text.starts_with("# ") {
                        crate::input::LineStyle {
                            text_scale: 2.,
                            height_scale: 2.5,
                            ..Default::default()
                        }
                    } else {
                        crate::input::LineStyle::default()
                    }
                }),
                cx,
            );
        });
        window.draw(cx).clear(cx);
        let state = editor.read(cx);
        let layout = state.last_layout.as_ref().unwrap();
        let lh = layout.line_height;
        let map = &state.display_map;

        // Vertical geometry: the heading row is 2.5 line heights tall.
        assert_eq!(map.display_row_top(1, lh), lh * 2.5);
        assert_eq!(map.display_row_top(2, lh), lh * 3.5);
        assert_eq!(map.display_row_height(0, lh), lh * 2.5);
        assert_eq!(map.display_row_height(1, lh), lh);
        assert_eq!(map.content_height(lh), lh * 4.5);
        assert_eq!(map.display_row_at_y(lh * 2.4, lh), 0);
        assert_eq!(map.display_row_at_y(lh * 2.6, lh), 1);
        assert_eq!(map.display_row_at_y(lh * 100., lh), 2);

        // Layout: the heading line is taller and shaped with the larger font.
        assert_eq!(layout.lines[0].size(lh).height, lh * 2.5);
        assert_eq!(layout.lines[1].size(lh).height, lh);
        let title = layout.lines[0].wrapped_lines[0].width;
        let body = TextElement::<EditorMode>::layout_range_corners(&(8..12), layout).unwrap();
        assert_eq!(body[0].top_left.y, layout.visible_top + lh * 2.5);
        assert_eq!(body[0].bottom_left.y - body[0].top_left.y, lh);
        let heading = TextElement::<EditorMode>::layout_range_corners(&(0..7), layout).unwrap();
        assert_eq!(heading[0].bottom_left.y - heading[0].top_left.y, lh * 2.5);
        assert!(title > layout.lines[1].wrapped_lines[0].width * 1.2);

        // Hit testing: the lower part of the heading row is still the heading line.
        let origin = state.last_bounds.unwrap().origin;
        let (ix, _) = state.index_for_mouse_position(origin + point(px(1.), lh * 2.2));
        assert!(ix <= 7, "{ix}");
        let (ix, _) = state.index_for_mouse_position(origin + point(px(1.), lh * 2.7));
        assert!((8..=12).contains(&ix), "{ix}");

        // The caret on the heading follows the heading's text size.
        let caret = layout.cursor_bounds.unwrap();
        assert!(caret.size.height > lh * 1.5, "{caret:?}");
    });
}

#[gpui::test]
fn line_blocks_reserve_space_below_their_line(cx: &mut TestAppContext) {
    let text = "body\n![img](a.png)\nmore";
    let (editor, window) = decoration_editor(cx, text, false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let rendered = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        let seen = rendered.clone();
        editor.update(cx, |state, cx| {
            state.set_line_blocks(
                |_, line: &str, _| {
                    if line.starts_with("![") {
                        px(100.)
                    } else {
                        px(0.)
                    }
                },
                move |block: &crate::input::LineBlockContext, _, _| {
                    seen.borrow_mut()
                        .push((block.line, block.text.clone(), block.size));
                    gpui::Empty.into_any_element()
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
        let state = editor.read(cx);
        let layout = state.last_layout.as_ref().unwrap();
        let lh = layout.line_height;
        let map = &state.display_map;

        // Only the image row grows, by the block height.
        assert_eq!(map.display_row_top(2, lh), lh * 2. + px(100.));
        assert_eq!(map.display_row_height(1, lh), lh + px(100.));
        assert_eq!(map.display_row_text_height(1, lh), lh);
        assert_eq!(map.content_height(lh), lh * 3. + px(100.));
        assert_eq!(layout.lines[1].size(lh).height, lh + px(100.));

        // The next line's ranges sit below the block.
        let more = TextElement::<EditorMode>::layout_range_corners(&(19..23), layout).unwrap();
        assert_eq!(more[0].top_left.y, layout.visible_top + lh * 2. + px(100.));

        // A click on the block lands on its line.
        let origin = state.last_bounds.unwrap().origin;
        let (ix, _) = state.index_for_mouse_position(origin + point(px(1.), lh * 1. + px(80.)));
        assert!((5..=18).contains(&ix), "{ix}");

        let rendered = rendered.borrow();
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].0, 1);
        assert_eq!(rendered[0].1.as_ref(), "![img](a.png)");
        assert_eq!(rendered[0].2.height, px(100.));
    });
}

#[gpui::test]
fn line_overlays_cover_ranges_except_on_selected_lines(cx: &mut TestAppContext) {
    let text = "- [ ] a\n- [x] b\nplain";
    let (editor, window) = decoration_editor(cx, text, false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let rendered = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        let seen = rendered.clone();
        editor.update(cx, |state, cx| {
            state.set_line_overlays(
                |_, line: &str| {
                    if line.starts_with("- [") {
                        vec![0..6]
                    } else {
                        vec![]
                    }
                },
                move |overlay: &crate::input::LineOverlayContext, _, _| {
                    seen.borrow_mut()
                        .push((overlay.line, overlay.range.clone(), overlay.size));
                    gpui::Empty.into_any_element()
                },
                cx,
            );
            state.set_selected_range(0..0, cx);
        });
        window.draw(cx).clear(cx);
        let lh = editor.read(cx).last_layout.as_ref().unwrap().line_height;
        {
            // The caret's line shows its text; the other task line is covered.
            let rendered = rendered.borrow();
            assert_eq!(rendered.len(), 1, "{rendered:?}");
            assert_eq!(rendered[0].0, 1);
            assert_eq!(rendered[0].1, 0..6);
            assert!(rendered[0].2.width > px(0.));
            assert_eq!(rendered[0].2.height, lh);
        }

        rendered.borrow_mut().clear();
        editor.update(cx, |state, cx| state.set_selected_range(18..18, cx));
        window.draw(cx).clear(cx);
        let lines: Vec<usize> = rendered.borrow().iter().map(|overlay| overlay.0).collect();
        assert_eq!(lines, [0, 1]);
    });
}

#[gpui::test]
fn replace_range_keeping_selections_leaves_the_caret(cx: &mut TestAppContext) {
    let text = "- [ ] a\nplain";
    let (editor, window) = decoration_editor(cx, text, false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, cx| {
        editor.update(cx, |state, cx| {
            state.set_selected_range(10..12, cx);
            state.replace_range_keeping_selections(3..4, "x", window, cx);
            assert_eq!(state.value().as_ref(), "- [x] a\nplain");
            assert_eq!(state.selected_range(), 10..12);
            // A longer replacement shifts the selection after it.
            state.replace_range_keeping_selections(3..4, "xx", window, cx);
            assert_eq!(state.selected_range(), 11..13);
            state.undo(&Undo, window, cx);
            assert_eq!(state.value().as_ref(), "- [x] a\nplain");
        });
    });
}

#[gpui::test]
fn line_style_blocks_add_to_measured_blocks(cx: &mut TestAppContext) {
    let text = "![img](a.png)\nbody";
    let (editor, window) = decoration_editor(cx, text, false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let rendered = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        let seen = rendered.clone();
        editor.update(cx, |state, cx| {
            state.set_line_style(
                Some(
                    |line: crate::input::LineContext<'_>| crate::input::LineStyle {
                        block_height: px(if line.next.is_some() { 20. } else { 0. }),
                        ..Default::default()
                    },
                ),
                cx,
            );
            state.set_line_blocks(
                |_, line: &str, _| {
                    if line.starts_with("![") {
                        px(100.)
                    } else {
                        px(0.)
                    }
                },
                move |block: &crate::input::LineBlockContext, _, _| {
                    seen.borrow_mut().push((block.line, block.size.height));
                    gpui::Empty.into_any_element()
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
        let state = editor.read(cx);
        let lh = state.last_layout.as_ref().unwrap().line_height;
        assert_eq!(state.display_map.display_row_height(0, lh), lh + px(120.));
        assert_eq!(state.display_map.display_row_height(1, lh), lh);
        assert_eq!(*rendered.borrow(), vec![(0, px(120.))]);
    });
}

#[gpui::test]
fn first_line_block_before_insets_the_document(cx: &mut TestAppContext) {
    let text = "title\nbody";
    let (editor, window) = decoration_editor(cx, text, false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let rendered = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        let seen = rendered.clone();
        editor.update(cx, |state, cx| {
            // Every line asks for a block above; only the first line's counts.
            state.set_line_style(
                Some(|_: crate::input::LineContext<'_>| crate::input::LineStyle {
                    block_before: px(40.),
                    ..Default::default()
                }),
                cx,
            );
            state.set_line_blocks(
                |_, _: &str, _| px(0.),
                move |block: &crate::input::LineBlockContext, _, _| {
                    seen.borrow_mut()
                        .push((block.line, block.above, block.size.height));
                    gpui::Empty.into_any_element()
                },
                cx,
            );
        });
        window.draw(cx).clear(cx);
        let state = editor.read(cx);
        let layout = state.last_layout.as_ref().unwrap();
        let lh = layout.line_height;
        let map = &state.display_map;
        assert_eq!(map.display_row_top(0, lh), px(40.));
        assert_eq!(map.display_row_top(1, lh), px(40.) + lh);
        assert_eq!(map.content_height(lh), px(40.) + lh * 2.);
        assert_eq!(map.display_row_at_y(px(40.) + lh * 1.5, lh), 1);
        assert_eq!(layout.visible_top, px(40.));

        // A click in the header lands on the first line; one on the second line stays there.
        let origin = state.last_bounds.unwrap().origin;
        let (ix, _) = state.index_for_mouse_position(origin + point(px(1.), px(10.)));
        assert!((0..=5).contains(&ix), "{ix}");
        let (ix, _) = state.index_for_mouse_position(origin + point(px(1.), px(40.) + lh * 1.5));
        assert!((6..=10).contains(&ix), "{ix}");

        assert_eq!(*rendered.borrow(), vec![(0, true, px(40.))]);
    });
}

#[gpui::test]
fn collapsed_lines_show_their_text_only_under_a_selection(cx: &mut TestAppContext) {
    let text = "body\n![img](a.png)\nmore";
    let (editor, window) = decoration_editor(cx, text, false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, cx| {
        editor.update(cx, |state, cx| {
            state.set_line_style(
                Some(
                    |line: crate::input::LineContext<'_>| crate::input::LineStyle {
                        collapsed: line.text.starts_with("!["),
                        ..Default::default()
                    },
                ),
                cx,
            );
            state.set_line_blocks(
                |_, line: &str, _| {
                    if line.starts_with("![") {
                        px(100.)
                    } else {
                        px(0.)
                    }
                },
                |_, _, _| gpui::Empty.into_any_element(),
                cx,
            );
            state.set_selected_range(0..0, cx);
        });
        window.draw(cx).clear(cx);
        let lh = editor.read(cx).last_layout.as_ref().unwrap().line_height;

        // The caret is elsewhere: only the block of the image line takes space.
        let state = editor.read(cx);
        assert_eq!(state.display_map.display_row_top(2, lh), lh + px(100.));
        let origin = state.last_bounds.unwrap().origin;
        // A click on the block puts the caret on the image line.
        let (ix, _) = state.index_for_mouse_position(origin + point(px(1.), lh + px(50.)));
        assert!((5..=18).contains(&ix), "{ix}");

        // The caret on the image line shows its text again.
        editor.update(cx, |state, cx| state.set_selected_range(ix..ix, cx));
        window.draw(cx).clear(cx);
        let state = editor.read(cx);
        assert_eq!(state.display_map.display_row_top(2, lh), lh * 2. + px(100.));

        // And hides it once the caret leaves.
        editor.update(cx, |state, cx| state.set_selected_range(20..20, cx));
        window.draw(cx).clear(cx);
        let state = editor.read(cx);
        assert_eq!(state.display_map.display_row_top(2, lh), lh + px(100.));
    });
}

#[gpui::test]
fn a_selection_reveals_the_whole_run_of_collapsed_lines(cx: &mut TestAppContext) {
    let text = "body\n|a|\n|-|\n|1|\nmore";
    let (editor, window) = decoration_editor(cx, text, false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, cx| {
        editor.update(cx, |state, cx| {
            state.set_line_style(
                Some(
                    |line: crate::input::LineContext<'_>| crate::input::LineStyle {
                        collapsed: line.text.starts_with('|'),
                        reveal_run: true,
                        ..Default::default()
                    },
                ),
                cx,
            );
            state.set_line_blocks(
                |row, _: &str, _| if row == 3 { px(80.) } else { px(0.) },
                |_, _, _| gpui::Empty.into_any_element(),
                cx,
            );
            state.set_selected_range(0..0, cx);
        });
        window.draw(cx).clear(cx);
        let lh = editor.read(cx).last_layout.as_ref().unwrap().line_height;

        // The caret is elsewhere: the run takes only its last line's block.
        let state = editor.read(cx);
        assert_eq!(state.display_map.display_row_top(4, lh), lh + px(80.));

        // The caret on the middle line shows all three lines.
        editor.update(cx, |state, cx| state.set_selected_range(10..10, cx));
        window.draw(cx).clear(cx);
        let state = editor.read(cx);
        assert_eq!(state.display_map.display_row_top(4, lh), lh * 4. + px(80.));

        // And hides them once the caret leaves.
        editor.update(cx, |state, cx| state.set_selected_range(0..0, cx));
        window.draw(cx).clear(cx);
        let state = editor.read(cx);
        assert_eq!(state.display_map.display_row_top(4, lh), lh + px(80.));
    });
}

#[gpui::test]
fn clicking_a_collapsed_block_reveals_its_line(cx: &mut TestAppContext) {
    let text = "body\n![img](a.png)\nmore";
    let (editor, window) = decoration_editor(cx, text, false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, cx| {
        editor.update(cx, |state, cx| {
            state.set_line_style(
                Some(
                    |line: crate::input::LineContext<'_>| crate::input::LineStyle {
                        collapsed: line.text.starts_with("!["),
                        ..Default::default()
                    },
                ),
                cx,
            );
            state.set_line_blocks(
                |_, line: &str, _| {
                    if line.starts_with("![") {
                        px(60.)
                    } else {
                        px(0.)
                    }
                },
                |_, _, _| gpui::Empty.into_any_element(),
                cx,
            );
            state.set_selected_range(0..0, cx);
        });
        window.draw(cx).clear(cx);
    });
    let (origin, lh) = cx.update(|_, cx| {
        let state = editor.read(cx);
        (
            state.last_bounds.unwrap().origin,
            state.last_layout.as_ref().unwrap().line_height,
        )
    });
    cx.simulate_click(
        origin + point(px(10.), lh + px(30.)),
        gpui::Modifiers::default(),
    );
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let state = editor.read(cx);
        let cursor = state.cursor();
        assert!((5..=18).contains(&cursor), "{cursor}");
        assert_eq!(state.display_map.display_row_top(2, lh), lh * 2. + px(60.));
    });
}

#[gpui::test]
fn styled_lines_soft_wrap_at_their_text_size(cx: &mut TestAppContext) {
    let words = "word ".repeat(12);
    let text = format!("# {words}\n{words}");
    let (editor, window) = decoration_editor(cx, &text, true);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, cx| {
        editor.update(cx, |state, cx| {
            state.set_line_style(
                Some(
                    |line: crate::input::LineContext<'_>| crate::input::LineStyle {
                        text_scale: if line.text.starts_with("# ") { 2. } else { 1. },
                        height_scale: 1.,
                        ..Default::default()
                    },
                ),
                cx,
            );
        });
        window.draw(cx).clear(cx);
        let state = editor.read(cx);
        let layout = state.last_layout.as_ref().unwrap();
        let heading_rows = layout.lines[0].wrapped_lines.len();
        let body_rows = layout.lines[1].wrapped_lines.len();
        assert!(heading_rows > body_rows, "{heading_rows} vs {body_rows}");
        let wrap_width = layout.wrap_width.unwrap();
        for line in &layout.lines[0].wrapped_lines {
            assert!(
                line.width <= wrap_width + px(1.),
                "{} > {wrap_width}",
                line.width
            );
        }
    });
}

/// An editor wrapping a long paragraph whose lines get `style`.
fn styled_paragraph(
    cx: &mut TestAppContext,
    text: &str,
    style: crate::input::LineStyle,
) -> (Entity<EditorState>, VisualTestContext) {
    let (editor, window) = decoration_editor(cx, text, true);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, cx| {
        editor.update(cx, |state, cx| {
            state.set_line_style(Some(move |_: crate::input::LineContext<'_>| style), cx);
        });
        window.draw(cx).clear(cx);
    });
    (editor, cx)
}

const PARAGRAPH: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";

#[gpui::test]
fn first_indent_moves_and_narrows_the_first_row(cx: &mut TestAppContext) {
    let (editor, mut cx) = styled_paragraph(
        cx,
        PARAGRAPH,
        crate::input::LineStyle {
            first_indent: px(30.),
            ..Default::default()
        },
    );
    cx.update(|_, cx| {
        let state = editor.read(cx);
        let layout = state.last_layout.as_ref().unwrap();
        let wrap = layout.wrap_width.unwrap();
        let line = &layout.lines[0];
        assert!(line.wrapped_lines.len() > 1);
        assert_eq!(line.line_indent(0), px(30.));
        assert_eq!(line.line_indent(1), px(0.));
        let first = &line.wrapped_lines[0];
        assert!(first.x_for_index(first.text.trim_end().len()) <= wrap - px(30.));
        let caret = line.position_for_index(0, layout, false).unwrap();
        assert_eq!(caret.x, px(30.));
    });
}

#[gpui::test]
fn centered_and_end_aligned_rows_share_the_free_width(cx: &mut TestAppContext) {
    for (align, share) in [
        (crate::input::LineAlign::Center, 0.5),
        (crate::input::LineAlign::End, 1.),
    ] {
        let (editor, mut cx) = styled_paragraph(
            cx,
            "short title",
            crate::input::LineStyle {
                align,
                ..Default::default()
            },
        );
        cx.update(|_, cx| {
            let state = editor.read(cx);
            let layout = state.last_layout.as_ref().unwrap();
            let wrap = layout.wrap_width.unwrap();
            let line = &layout.lines[0];
            let width = line.wrapped_lines[0].width;
            let expected = (wrap - width) * share;
            assert!(
                (line.line_indent(0) - expected).abs() < px(0.5),
                "{align:?}"
            );
            let caret = line.position_for_index(0, layout, false).unwrap();
            assert_eq!(caret.x, line.line_indent(0));
        });
    }
}

#[gpui::test]
fn justified_rows_reach_the_wrap_width_and_hit_test_back(cx: &mut TestAppContext) {
    let (editor, mut cx) = styled_paragraph(
        cx,
        PARAGRAPH,
        crate::input::LineStyle {
            align: crate::input::LineAlign::Justify,
            ..Default::default()
        },
    );
    cx.update(|_, cx| {
        let state = editor.read(cx);
        let layout = state.last_layout.as_ref().unwrap();
        let wrap = layout.wrap_width.unwrap();
        let line = &layout.lines[0];
        let rows = line.wrapped_lines.len();
        assert!(rows > 1);
        let mut start = 0;
        for (ix, row) in line.wrapped_lines.iter().enumerate() {
            let end = row.text.trim_end().len();
            let right = line.line_indent(ix) + row.x_for_index(end);
            if ix + 1 < rows {
                assert!(
                    (right - wrap).abs() < px(0.5),
                    "row {ix}: {right:?} vs {wrap:?}"
                );
            } else {
                assert!(right < wrap, "the last row stays ragged");
            }
            // Every caret position in a stretched row maps back to its own offset.
            let stretched = if ix + 1 < rows { row.text.len() } else { 0 };
            for offset in row.text[..stretched].char_indices().map(|(i, _)| start + i) {
                let pos = line.position_for_index(offset, layout, false).unwrap();
                let (hit, _) = line
                    .closest_index_for_position(pos + point(px(0.1), px(1.)), layout)
                    .unwrap();
                assert_eq!(hit, offset, "row {ix}");
            }
            start += row.len;
        }
    });
}

#[gpui::test]
fn space_after_reserves_height_without_a_block(cx: &mut TestAppContext) {
    let (editor, mut cx) = styled_paragraph(
        cx,
        "one\ntwo",
        crate::input::LineStyle {
            space_after: px(12.),
            ..Default::default()
        },
    );
    cx.update(|_, cx| {
        let state = editor.read(cx);
        let lh = state.last_layout.as_ref().unwrap().line_height;
        assert_eq!(state.display_map.display_row_top(1, lh), lh + px(12.));
        assert_eq!(state.display_map.display_row_text_height(0, lh), lh);
    });
}

#[gpui::test]
fn neighbors_are_restyled_when_a_line_changes(cx: &mut TestAppContext) {
    let (editor, window) = decoration_editor(cx, "body\n\nmore", false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    cx.update(|window, cx| {
        editor.update(cx, |state, cx| {
            // A blank line before a heading gets space below it.
            state.set_line_style(
                Some(
                    |line: crate::input::LineContext<'_>| crate::input::LineStyle {
                        space_after: if line.text.is_empty()
                            && line.next.is_some_and(|next| next.starts_with('#'))
                        {
                            px(20.)
                        } else {
                            px(0.)
                        },
                        ..Default::default()
                    },
                ),
                cx,
            );
            state.set_selected_range(0..0, cx);
        });
        window.draw(cx).clear(cx);
        let lh = editor.read(cx).last_layout.as_ref().unwrap().line_height;
        assert_eq!(editor.read(cx).display_map.display_row_top(2, lh), lh * 2.);

        // Turning the next line into a heading restyles the blank line before it.
        editor.update(cx, |state, cx| {
            state.set_selected_range(6..6, cx);
            state.replace("# ", window, cx);
        });
        window.draw(cx).clear(cx);
        assert_eq!(
            editor.read(cx).display_map.display_row_top(2, lh),
            lh * 2. + px(20.)
        );
    });
}

#[gpui::test]
fn secondary_click_goes_to_the_host_before_placing_the_caret(cx: &mut TestAppContext) {
    let text = "first line\nsecond line";
    let (editor, window) = decoration_editor(cx, text, false);
    let mut cx = VisualTestContext::from_window(window.into(), cx);
    let clicks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    cx.update(|window, cx| {
        let seen = clicks.clone();
        editor.update(cx, |state, cx| {
            state.set_secondary_click(move |text, offset, _, _| {
                seen.borrow_mut().push((text.to_string(), offset));
                true
            });
            state.set_selected_range(0..0, cx);
        });
        window.draw(cx).clear(cx);
    });
    let (origin, lh) = cx.update(|_, cx| {
        let state = editor.read(cx);
        (
            state.last_bounds.unwrap().origin,
            state.last_layout.as_ref().unwrap().line_height,
        )
    });
    let on_second_line = origin + point(px(10.), lh + lh / 2.);

    // ⌘ (macOS) / Ctrl click: the host takes it and the caret stays.
    cx.simulate_click(on_second_line, gpui::Modifiers::secondary_key());
    cx.update(|_, cx| {
        let clicks = clicks.borrow();
        assert_eq!(clicks.len(), 1);
        assert_eq!(clicks[0].0, text);
        assert!((11..=text.len()).contains(&clicks[0].1), "{}", clicks[0].1);
        assert_eq!(editor.read(cx).cursor(), 0);
    });

    // A plain click places the caret as usual.
    cx.simulate_click(on_second_line, gpui::Modifiers::default());
    cx.update(|_, cx| {
        assert_eq!(clicks.borrow().len(), 1);
        assert!(editor.read(cx).cursor() >= 11);
    });
}
