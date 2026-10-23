//! PDF 내보내기: 미리보기가 배치한 페이지를 그대로 PDF에 찍는다(ADR-0014).
//!
//! 배치는 미리보기(`app/book_preview/layout.rs`)가 gpui 글자 측정으로 한다. 여기서는 그 결과
//! (페이지마다 글자 조각·이미지·선의 위치, 단위 pt)를 받아 krilla로 쓴다. gpui 타입은 쓰지 않는다.
//!
//! - 서체: 시스템 서체를 이름·굵기·기울임으로 찾아(fontdb) 넣는다. krilla가 쓰는 글자만 남긴다(subset).
//! - 서체에 없는 글자는 대체 서체(한글·한자·가나)로 그린다. 조각 안의 글자 위치는 서체 글자 폭으로 잇는다.
//! - 글자 조각의 시작 x는 배치 그대로라 양쪽 정렬·들여쓰기가 미리보기와 같다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use krilla::Document;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::image::Image;
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule};
use krilla::surface::Surface;
use krilla::text::{Font, TextDirection};

use super::ExportError;

/// 내보낼 문서.
pub struct PdfDocument {
    pub title: String,
    pub authors: Vec<String>,
    pub pages: Vec<PdfPage>,
}

/// 한 페이지. 길이는 pt, 원점은 왼쪽 위.
pub struct PdfPage {
    pub width: f32,
    pub height: f32,
    pub items: Vec<PdfItem>,
}

pub enum PdfItem {
    /// 한 줄(또는 양쪽 정렬된 행의 한 조각) 글자. `top`은 줄 칸 위쪽이다.
    Text {
        x: f32,
        top: f32,
        line_height: f32,
        size: f32,
        text: String,
        runs: Vec<PdfRun>,
    },
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        path: PathBuf,
        /// 칸을 꽉 채운다(넘치는 부분은 자른다). 아니면 칸에 그대로 맞춘다.
        cover: bool,
    },
    Rule {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: [u8; 3],
    },
}

/// 같은 모양의 글자 조각(바이트 길이).
#[derive(Clone, Debug)]
pub struct PdfRun {
    pub len: usize,
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub color: [u8; 3],
    pub strike: bool,
}

/// 서체에 없는 글자를 그릴 대체 서체(앞에서부터 찾는다).
const FALLBACK_FAMILIES: [&str; 6] = [
    "Apple SD Gothic Neo",
    "Hiragino Sans",
    "PingFang SC",
    "Noto Sans CJK KR",
    "Malgun Gothic",
    "Arial Unicode MS",
];

/// 이름을 찾지 못한 서체(시스템 UI 서체 등) 대신 쓸 서체.
const DEFAULT_FAMILIES: [&str; 3] = ["Apple SD Gothic Neo", "Helvetica Neue", "Malgun Gothic"];

/// PDF로 쓴다.
pub fn write(document: &PdfDocument, target: &Path) -> Result<(), ExportError> {
    let bytes = render(document)?;
    std::fs::write(target, bytes)?;
    Ok(())
}

/// PDF 바이트를 만든다.
pub fn render(document: &PdfDocument) -> Result<Vec<u8>, ExportError> {
    let mut fonts = Fonts::new();
    let mut images: HashMap<PathBuf, Option<Image>> = HashMap::new();
    let mut pdf = Document::new();
    let mut metadata = Metadata::new().title(document.title.clone());
    if !document.authors.is_empty() {
        metadata = metadata.authors(document.authors.clone());
    }
    pdf.set_metadata(metadata);

    for page in &document.pages {
        let settings =
            PageSettings::from_wh(page.width.max(1.), page.height.max(1.)).ok_or_else(|| {
                ExportError::InvalidProject(t!("export.error.page_size").into_owned())
            })?;
        let mut pdf_page = pdf.start_page_with(settings);
        let mut surface = pdf_page.surface();
        for item in &page.items {
            match item {
                PdfItem::Text {
                    x,
                    top,
                    line_height,
                    size,
                    text,
                    runs,
                } => draw_text(
                    &mut surface,
                    &mut fonts,
                    *x,
                    *top,
                    *line_height,
                    *size,
                    text,
                    runs,
                ),
                PdfItem::Rule {
                    x,
                    y,
                    width,
                    height,
                    color,
                } => fill_rect(&mut surface, *x, *y, *width, height.max(0.5), *color),
                PdfItem::Image {
                    x,
                    y,
                    width,
                    height,
                    path,
                    cover,
                } => {
                    let image = images
                        .entry(path.clone())
                        .or_insert_with(|| load_image(path))
                        .clone();
                    if let Some(image) = image {
                        draw_image(&mut surface, image, *x, *y, *width, *height, *cover);
                    }
                }
            }
        }
        surface.finish();
        pdf_page.finish();
    }
    pdf.finish().map_err(|err| {
        ExportError::InvalidProject(t!("export.error.pdf", err = format!("{err:?}")).into_owned())
    })
}

fn fill_rect(surface: &mut Surface, x: f32, y: f32, width: f32, height: f32, color: [u8; 3]) {
    let Some(rect) = Rect::from_xywh(x, y, width.max(0.01), height.max(0.01)) else {
        return;
    };
    let mut builder = PathBuilder::new();
    builder.push_rect(rect);
    let Some(path) = builder.finish() else {
        return;
    };
    surface.set_fill(Some(fill(color)));
    surface.draw_path(&path);
}

fn fill(color: [u8; 3]) -> Fill {
    Fill {
        paint: rgb::Color::new(color[0], color[1], color[2]).into(),
        opacity: NormalizedF32::ONE,
        rule: FillRule::default(),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    surface: &mut Surface,
    fonts: &mut Fonts,
    x: f32,
    top: f32,
    line_height: f32,
    size: f32,
    text: &str,
    runs: &[PdfRun],
) {
    // 기준선: 줄 칸 안에서 글자 높이(ascent + descent)를 세로 가운데에 둔다(gpui와 같다).
    let Some(first) = runs.first() else {
        return;
    };
    let Some(primary) = fonts.face(&first.family, first.weight, first.italic) else {
        return;
    };
    let (ascent, descent) = fonts.metrics(primary);
    let baseline = top + (line_height - (ascent + descent) * size) / 2. + ascent * size;

    let mut pen = x;
    let mut start = 0;
    for run in runs {
        let end = (start + run.len).min(text.len());
        let Some(slice) = text.get(start..end) else {
            break;
        };
        start = end;
        let Some(face) = fonts.face(&run.family, run.weight, run.italic) else {
            continue;
        };
        let run_start = pen;
        // 글자마다 그릴 서체를 정해 같은 서체끼리 묶어 그린다.
        let mut segment = String::new();
        let mut segment_face = face;
        for c in slice.chars() {
            let chosen = fonts.face_for_char(face, c);
            if chosen != segment_face && !segment.is_empty() {
                pen += fonts.draw(
                    surface,
                    segment_face,
                    &segment,
                    pen,
                    baseline,
                    size,
                    run.color,
                );
                segment.clear();
            }
            segment_face = chosen;
            segment.push(c);
        }
        if !segment.is_empty() {
            pen += fonts.draw(
                surface,
                segment_face,
                &segment,
                pen,
                baseline,
                size,
                run.color,
            );
        }
        if run.strike {
            let thickness = size * 0.06;
            fill_rect(
                surface,
                run_start,
                baseline - size * 0.3 - thickness / 2.,
                pen - run_start,
                thickness,
                run.color,
            );
        }
    }
}

fn draw_image(
    surface: &mut Surface,
    image: Image,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    cover: bool,
) {
    let (natural_w, natural_h) = image.size();
    let (natural_w, natural_h) = (natural_w as f32, natural_h as f32);
    if natural_w <= 0. || natural_h <= 0. || width <= 0. || height <= 0. {
        return;
    }
    // 칸 비율과 이미지 비율이 다르면 cover는 넘치는 쪽을 자르고, 아니면 칸 안에 맞춰 가운데 둔다.
    let scale = if cover {
        (width / natural_w).max(height / natural_h)
    } else {
        (width / natural_w).min(height / natural_h)
    };
    let (draw_w, draw_h) = (natural_w * scale, natural_h * scale);
    let (dx, dy) = (x + (width - draw_w) / 2., y + (height - draw_h) / 2.);
    let Some(size) = Size::from_wh(draw_w, draw_h) else {
        return;
    };
    let clip = Rect::from_xywh(x, y, width, height).and_then(|rect| {
        let mut builder = PathBuilder::new();
        builder.push_rect(rect);
        builder.finish()
    });
    if let Some(clip) = &clip {
        surface.push_clip_path(clip, &FillRule::NonZero);
    }
    surface.push_transform(&Transform::from_translate(dx, dy));
    surface.draw_image(image, size);
    surface.pop();
    if clip.is_some() {
        surface.pop();
    }
}

/// 이미지 파일을 읽는다. PNG·JPEG·GIF·WebP는 그대로 넣고, 나머지는 PNG로 바꿔 넣는다.
fn load_image(path: &Path) -> Option<Image> {
    let data = std::fs::read(path).ok()?;
    let direct = if data.starts_with(b"\x89PNG") {
        Image::from_png(Arc::new(data.clone()).into(), true).ok()
    } else if data.starts_with(&[0xFF, 0xD8]) {
        Image::from_jpeg(Arc::new(data.clone()).into(), true).ok()
    } else if data.starts_with(b"GIF8") {
        Image::from_gif(Arc::new(data.clone()).into(), true).ok()
    } else if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Image::from_webp(Arc::new(data.clone()).into(), true).ok()
    } else {
        None
    };
    direct.or_else(|| {
        let decoded = image::load_from_memory(&data).ok()?;
        let mut png = Vec::new();
        decoded
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .ok()?;
        Image::from_png(Arc::new(png).into(), true).ok()
    })
}

/// 읽은 서체 하나.
struct Face {
    font: Font,
    data: Arc<Vec<u8>>,
    index: u32,
    /// ascent, descent(양수), 글자 폭 단위(em 대비).
    ascent: f32,
    descent: f32,
    units_per_em: f32,
}

type FaceId = usize;

/// 시스템 서체 찾기와 읽은 서체 모음.
struct Fonts {
    db: fontdb::Database,
    faces: Vec<Face>,
    by_source: HashMap<fontdb::ID, FaceId>,
    by_query: HashMap<(String, u16, bool), Option<FaceId>>,
    fallbacks: Option<Vec<FaceId>>,
    /// (서체, 글자) → 글자가 있는지.
    coverage: HashMap<(FaceId, char), bool>,
}

impl Fonts {
    fn new() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Self {
            db,
            faces: Vec::new(),
            by_source: HashMap::new(),
            by_query: HashMap::new(),
            fallbacks: None,
            coverage: HashMap::new(),
        }
    }

    /// 이름·굵기·기울임에 가장 가까운 서체. 이름을 못 찾으면 기본 서체.
    fn face(&mut self, family: &str, weight: u16, italic: bool) -> Option<FaceId> {
        let key = (family.to_string(), weight, italic);
        if let Some(found) = self.by_query.get(&key) {
            return *found;
        }
        let found = std::iter::once(family)
            .chain(DEFAULT_FAMILIES)
            .find_map(|name| self.query(name, weight, italic));
        self.by_query.insert(key, found);
        found
    }

    fn query(&mut self, family: &str, weight: u16, italic: bool) -> Option<FaceId> {
        let families = [fontdb::Family::Name(family)];
        let query = fontdb::Query {
            families: &families,
            weight: fontdb::Weight(weight),
            style: if italic {
                fontdb::Style::Italic
            } else {
                fontdb::Style::Normal
            },
            stretch: fontdb::Stretch::Normal,
        };
        let id = self.db.query(&query)?;
        // 다른 서체로 대신 찾아 준 결과는 받지 않는다(이름이 같아야 한다).
        let matches = self.db.face(id).is_some_and(|face| {
            face.families
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case(family))
        });
        if !matches {
            return None;
        }
        self.load(id)
    }

    fn load(&mut self, id: fontdb::ID) -> Option<FaceId> {
        if let Some(face) = self.by_source.get(&id) {
            return Some(*face);
        }
        let (data, index) = self
            .db
            .with_face_data(id, |data, index| (data.to_vec(), index))?;
        let data = Arc::new(data);
        let parsed = ttf_parser::Face::parse(&data, index).ok()?;
        let units_per_em = f32::from(parsed.units_per_em());
        let ascent = f32::from(parsed.ascender()) / units_per_em;
        let descent = -f32::from(parsed.descender()) / units_per_em;
        let font = Font::new(data.clone().into(), index)?;
        self.faces.push(Face {
            font,
            data,
            index,
            ascent,
            descent,
            units_per_em,
        });
        let face = self.faces.len() - 1;
        self.by_source.insert(id, face);
        Some(face)
    }

    fn metrics(&self, face: FaceId) -> (f32, f32) {
        let face = &self.faces[face];
        (face.ascent, face.descent)
    }

    fn has_glyph(&mut self, face: FaceId, c: char) -> bool {
        if let Some(found) = self.coverage.get(&(face, c)) {
            return *found;
        }
        let info = &self.faces[face];
        let found = ttf_parser::Face::parse(&info.data, info.index)
            .ok()
            .and_then(|parsed| parsed.glyph_index(c))
            .is_some();
        self.coverage.insert((face, c), found);
        found
    }

    /// 글자를 그릴 서체: 그 서체에 있으면 그대로, 없으면 글자가 있는 첫 대체 서체.
    fn face_for_char(&mut self, face: FaceId, c: char) -> FaceId {
        if c.is_whitespace() || c.is_control() || self.has_glyph(face, c) {
            return face;
        }
        if self.fallbacks.is_none() {
            let fallbacks = FALLBACK_FAMILIES
                .iter()
                .filter_map(|name| self.query(name, 400, false))
                .collect();
            self.fallbacks = Some(fallbacks);
        }
        let fallbacks = self.fallbacks.clone().unwrap_or_default();
        fallbacks
            .into_iter()
            .find(|fallback| self.has_glyph(*fallback, c))
            .unwrap_or(face)
    }

    /// 한 서체로 글자를 그리고 그린 폭을 돌려준다.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        surface: &mut Surface,
        face: FaceId,
        text: &str,
        x: f32,
        baseline: f32,
        size: f32,
        color: [u8; 3],
    ) -> f32 {
        let info = &self.faces[face];
        surface.set_fill(Some(fill(color)));
        surface.draw_text(
            Point::from_xy(x, baseline),
            info.font.clone(),
            size,
            text,
            false,
            TextDirection::Auto,
        );
        let Ok(parsed) = ttf_parser::Face::parse(&info.data, info.index) else {
            return 0.;
        };
        let units: f32 = text
            .chars()
            .map(|c| {
                parsed
                    .glyph_index(c)
                    .and_then(|glyph| parsed.glyph_hor_advance(glyph))
                    .map_or(0., f32::from)
            })
            .sum();
        units / info.units_per_em * size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_pages_with_text_rules_and_fallback_glyphs() {
        let run = |len| PdfRun {
            len,
            family: "존재하지 않는 서체".into(),
            weight: 400,
            italic: false,
            color: [20, 20, 20],
            strike: false,
        };
        let text = "가나 Abc 漢字";
        let document = PdfDocument {
            title: "시험".into(),
            authors: vec!["지은이".into()],
            pages: vec![
                PdfPage {
                    width: 420.,
                    height: 595.,
                    items: vec![
                        PdfItem::Text {
                            x: 50.,
                            top: 50.,
                            line_height: 16.,
                            size: 10.,
                            text: text.into(),
                            runs: vec![run(text.len())],
                        },
                        PdfItem::Rule {
                            x: 50.,
                            y: 80.,
                            width: 100.,
                            height: 0.5,
                            color: [180, 180, 180],
                        },
                    ],
                },
                PdfPage {
                    width: 420.,
                    height: 595.,
                    items: Vec::new(),
                },
            ],
        };
        let bytes = render(&document).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
    }
}
