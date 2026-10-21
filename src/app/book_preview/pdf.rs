//! 미리보기 배치 → PDF(ADR-0014). 화면에 그린 페이지를 같은 위치·서체로 PDF에 옮긴다.
//!
//! 배치는 gpui 글자 측정이 필요해 UI 스레드에서 하고, PDF 쓰기(서체 읽기·압축)는 백그라운드에서 한다.

use gpui_kit::gpui::*;
use std::path::PathBuf;

use super::layout::{self, BookLayout, ImageFile, Item};
use super::{Paper, layout_fonts, load_selection, load_source};
use crate::export::pdf::{PdfDocument, PdfItem, PdfPage, PdfRun};

/// 배치한 페이지를 PDF 문서로 옮긴다. 웹 이미지는 넣지 않는다.
pub(super) fn document(layout: &BookLayout, title: &str, authors: &[String]) -> PdfDocument {
    let setup = layout.setup;
    let pages = layout
        .pages
        .iter()
        .map(|page| PdfPage {
            width: f32::from(setup.width),
            height: f32::from(setup.height),
            items: page.items.iter().filter_map(item).collect(),
        })
        .collect();
    PdfDocument {
        title: title.to_string(),
        authors: authors.to_vec(),
        pages,
    }
}

fn item(item: &Item) -> Option<PdfItem> {
    Some(match item {
        Item::Text {
            origin,
            line_height,
            font_size,
            text,
            runs,
        } => PdfItem::Text {
            x: f32::from(origin.x),
            top: f32::from(origin.y),
            line_height: f32::from(*line_height),
            size: f32::from(*font_size),
            text: text.to_string(),
            runs: runs
                .iter()
                .map(|run| PdfRun {
                    len: run.len,
                    family: run.font.family.to_string(),
                    weight: run.font.weight.0.round().clamp(100., 900.) as u16,
                    italic: run.font.style != FontStyle::Normal,
                    color: rgb(run.color),
                    strike: run.strikethrough.is_some(),
                })
                .collect(),
        },
        Item::Image {
            bounds,
            source: ImageFile::File { path, .. },
            cover,
        } => PdfItem::Image {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
            path: path.clone(),
            cover: *cover,
        },
        Item::Image {
            source: ImageFile::Url(_),
            ..
        } => return None,
        Item::Rule { bounds, color } => PdfItem::Rule {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
            color: rgb(*color),
        },
    })
}

fn rgb(color: Hsla) -> [u8; 3] {
    let color = color.to_rgb();
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    [channel(color.r), channel(color.g), channel(color.b)]
}

/// 원고를 읽어 판형대로 배치하고 PDF로 쓴다(컴파일 대화상자). 미리보기 창과 같은 배치다.
/// `selection`이 있으면 (라이브러리 루트, 고른 그룹·시트)만 내보낸다.
pub(crate) fn export_pdf(
    project: PathBuf,
    selection: Option<(PathBuf, Vec<PathBuf>)>,
    paper: Paper,
    generate_toc: bool,
    target: PathBuf,
    window: &Window,
    cx: &App,
) -> Task<Result<(), String>> {
    window.spawn(cx, async move |cx| {
        let source = cx
            .background_executor()
            .spawn(async move {
                match selection {
                    Some((root, selected)) => load_selection(&root, &selected),
                    None => load_source(&project),
                }
            })
            .await
            .map_err(|err| err.to_string())?;
        let pdf = cx
            .update(|window, cx| {
                let fonts = layout_fonts(cx);
                let layout = layout::layout_book(
                    &source,
                    paper.setup(),
                    &fonts,
                    window.text_system(),
                    generate_toc,
                );
                document(&layout, &source.title, &source.authors)
            })
            .map_err(|err| err.to_string())?;
        cx.background_executor()
            .spawn(async move { crate::export::pdf::write(&pdf, &target) })
            .await
            .map_err(|err| err.to_string())
    })
}
