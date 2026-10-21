//! ePub 빌더: 프로젝트 원고를 표준 전자책(.epub)으로 패키징한다.
//!
//! - 제목 스타일(ADR-0009):
//!   - 별도 제목 페이지(`Placement::Page`, 도비라): 독립된 단독 페이지로 분리 및 세로 위치(`vertical_align`) 적용.
//!   - 제목 숨김(`Placement::Hidden`): 화면에서는 숨기되 목차에만 보존.
//!   - 본문 첫머리(`Placement::Inline`): 본문과 연결 및 새 페이지 시작(`page_break`) 처리.
//!   - 접두사 및 번호 매기기(`prefix`, `{n}`): 부/장/절별 자동 번호 매김.
//!   - 목차 포함(`in_toc`): 목차에서 뺄지 제어. 번호와 접두사는 그대로 붙는다.
//! - 독서 앱 호환: 별도의 불필요한 nav 페이지 없이 전자책 내장 목차(`toc.ncx`)가 단독으로 차례를 담당.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Seek, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use super::{ExportError, ExportProject, load_project};
use crate::library::{Align, Placement, ResolvedStyle, StyleTarget, VerticalAlign};
use crate::markdown::{Block, Inline, Marks, Rich};

/// XML 특수문자를 이스케이프한다.
fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// 목차 항목에 쓸 글자. 개행은 스페이스로 바꾼다.
fn toc_label(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// 이미지 확장자에 맞는 MIME 타입을 돌려준다.
fn mime_type_for_extension(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// 목차 항목.
#[derive(Debug, Clone)]
struct TocItem {
    id: String,
    title: String,
    href: String,
}

/// XHTML 섹션 하나 (독립된 페이지 또는 장 파일).
struct Section {
    id: String,
    filename: String,
    title: String,
    html_body: String,
}

/// EPUB 아카이브에 포함할 이미지 에셋.
struct ImageAsset {
    id: String,
    zip_path: String,
    source_path: PathBuf,
    media_type: &'static str,
    is_cover: bool,
}

/// ePub 생성 상태.
struct EpubBuilder<'a> {
    project: &'a ExportProject,
    sections: Vec<Section>,
    current_section: Option<Section>,
    section_counter: usize,
    /// 목차 포함 여부와 무관하게 제목마다 고유 앵커를 만든다.
    heading_counter: usize,
    image_counter: usize,
    images: Vec<ImageAsset>,
    image_dest_map: HashMap<String, String>,
    toc_items: Vec<TocItem>,
    /// 부(0), 장(1), 절(2) 카운터.
    numbers: [usize; 3],
}

impl<'a> EpubBuilder<'a> {
    fn new(project: &'a ExportProject) -> Self {
        Self {
            project,
            sections: Vec::new(),
            current_section: None,
            section_counter: 0,
            heading_counter: 0,
            image_counter: 0,
            images: Vec::new(),
            image_dest_map: HashMap::new(),
            toc_items: Vec::new(),
            numbers: [0; 3],
        }
    }

    /// 현재 작업 중인 섹션이 있고 비어있지 않다면 마감한다.
    fn finish_section(&mut self) {
        if let Some(sec) = self.current_section.take() {
            if !sec.html_body.trim().is_empty() {
                self.sections.push(sec);
            }
        }
    }

    /// 현재 섹션을 가져오거나, 없으면 새로 만든다.
    fn ensure_section(&mut self, default_title: &str) -> &mut Section {
        if self.current_section.is_none() {
            self.section_counter += 1;
            let filename = format!("text/section_{:04}.xhtml", self.section_counter);
            let id = format!("sec_{:04}", self.section_counter);
            self.current_section = Some(Section {
                id,
                filename,
                title: default_title.to_string(),
                html_body: String::new(),
            });
        }
        self.current_section.as_mut().unwrap()
    }

    /// 제목 블록을 스타일 설정에 맞춰 렌더링한다.
    fn render_heading(&mut self, level: u8, text: &Rich) {
        self.heading_counter += 1;
        let anchor_id = format!("h-{}", self.heading_counter);
        let raw_title = crate::markdown::rich_text(text).trim().to_string();
        let target = StyleTarget::for_heading(level);

        if let Some(target) = target {
            let resolved = self.project.styles.resolve(target);
            let depth = match target {
                StyleTarget::Part => 0,
                StyleTarget::Chapter => 1,
                _ => 2,
            };

            // 번호와 접두사는 목차 포함 여부와 무관하게 붙인다. 다만 아예 그리지도 않고
            // 목차에도 넣지 않는 제목(숨김 + 목차 제외)은 번호를 쓰지 않는다.
            let numbered = resolved.placement != Placement::Hidden || resolved.in_toc;
            // 본문 제목과 목차 제목을 따로 만든다. 목차에서는 접두사의 개행을 스페이스로 바꾼다.
            let (display_title, toc_title) = if numbered {
                self.numbers[depth] += 1;
                (
                    resolved.title_text(self.numbers[depth], &raw_title),
                    resolved.toc_title_text(self.numbers[depth], &raw_title),
                )
            } else {
                (raw_title.clone(), raw_title.clone())
            };

            let target_class = match target {
                StyleTarget::Part => "title-part",
                StyleTarget::Chapter => "title-chapter",
                StyleTarget::Section => "title-section",
                _ => "title-heading",
            };

            match resolved.placement {
                Placement::Hidden => {
                    // 제목 숨김: 본문에는 나타나지 않음. 목차 포함 설정 시 목차에만 등록
                    if resolved.in_toc {
                        let filename = self.ensure_section(&display_title).filename.clone();
                        let href = format!("{}#{}", filename, anchor_id);
                        self.toc_items.push(TocItem {
                            id: anchor_id.clone(),
                            title: toc_title,
                            href,
                        });
                        self.ensure_section("").html_body.push_str(&format!("<span id=\"{anchor_id}\"></span>\n"));
                    }
                }
                Placement::Page => {
                    // 별도 제목 페이지(도비라): 이전 섹션을 마감하고 단독 제목 섹션 생성
                    self.finish_section();

                    self.section_counter += 1;
                    let filename = format!("text/section_{:04}.xhtml", self.section_counter);
                    let id = format!("sec_{:04}", self.section_counter);

                    let v_align_class = match resolved.vertical_align {
                        VerticalAlign::Top => "vertical-top",
                        VerticalAlign::Center => "vertical-center",
                        VerticalAlign::Bottom => "vertical-bottom",
                    };

                    let heading_html = render_rich_with_prefix(text, &resolved, self.numbers[depth]);
                    let page_sec = Section {
                        id,
                        filename: filename.clone(),
                        title: display_title.clone(),
                        html_body: format!(
                            "<div class=\"title-page {v_align_class}\">\n  <h{level} class=\"{target_class}\" id=\"{anchor_id}\">{heading_html}</h{level}>\n</div>\n"
                        ),
                    };

                    if resolved.in_toc {
                        let href = format!("{}#{}", filename, anchor_id);
                        self.toc_items.push(TocItem {
                            id: anchor_id,
                            title: toc_title,
                            href,
                        });
                    }

                    // 단독 제목 페이지이므로 즉시 닫아 다음 본문이 새 파일에서 시작하도록 함
                    self.sections.push(page_sec);
                }
                Placement::Inline => {
                    // 본문 첫머리: 새 페이지 시작(page_break)이 설정되어 있으면 이전 섹션 마감
                    if resolved.page_break {
                        self.finish_section();
                    }

                    let heading_html = render_rich_with_prefix(text, &resolved, self.numbers[depth]);
                    let filename = self.ensure_section(&display_title).filename.clone();
                    if resolved.in_toc {
                        let href = format!("{}#{}", filename, anchor_id);
                        self.toc_items.push(TocItem {
                            id: anchor_id.clone(),
                            title: toc_title,
                            href,
                        });
                    }
                    self.ensure_section("").html_body.push_str(&format!(
                        "<h{level} class=\"{target_class}\" id=\"{anchor_id}\">{heading_html}</h{level}>\n"
                    ));
                }
            }
        } else {
            // H4~H6 등 제목 스타일 대상이 아닌 일반 헤딩
            let filename = self.ensure_section(&raw_title).filename.clone();
            let href = format!("{}#{}", filename, anchor_id);
            self.toc_items.push(TocItem {
                id: anchor_id.clone(),
                title: raw_title,
                href,
            });
            self.ensure_section("").html_body.push_str(&format!(
                "<h{level} id=\"{anchor_id}\">{}</h{level}>\n",
                render_rich(text)
            ));
        }
    }

    /// 일반 블록 렌더링.
    fn render_block(&mut self, block: &Block, sheet_dir: &Path) {
        match block {
            Block::Heading { level, text } => {
                self.render_heading(*level, text);
            }
            Block::Paragraph(rich) => {
                let sec = self.ensure_section(&self.project.name);
                sec.html_body.push_str("<p>");
                sec.html_body.push_str(&render_rich(rich));
                sec.html_body.push_str("</p>\n");
            }
            Block::Quote(blocks) => {
                let sec = self.ensure_section(&self.project.name);
                sec.html_body.push_str("<blockquote>\n");
                for b in blocks {
                    self.render_block(b, sheet_dir);
                }
                let sec = self.ensure_section(&self.project.name);
                sec.html_body.push_str("</blockquote>\n");
            }
            Block::List {
                ordered,
                start,
                items,
            } => {
                let tag = if *ordered { "ol" } else { "ul" };
                let sec = self.ensure_section(&self.project.name);
                if *ordered && *start != 1 {
                    sec.html_body.push_str(&format!("<{tag} start=\"{start}\">\n"));
                } else {
                    sec.html_body.push_str(&format!("<{tag}>\n"));
                }
                for item in items {
                    let sec = self.ensure_section(&self.project.name);
                    sec.html_body.push_str("<li>\n");
                    for b in item {
                        self.render_block(b, sheet_dir);
                    }
                    let sec = self.ensure_section(&self.project.name);
                    sec.html_body.push_str("</li>\n");
                }
                let sec = self.ensure_section(&self.project.name);
                sec.html_body.push_str(&format!("</{tag}>\n"));
            }
            Block::Code(code) => {
                let sec = self.ensure_section(&self.project.name);
                sec.html_body.push_str("<pre><code>");
                sec.html_body.push_str(&escape_xml(code));
                sec.html_body.push_str("</code></pre>\n");
            }
            Block::Image(img) => {
                let resolved_path = resolve_image_path(sheet_dir, &self.project.dir, &img.destination);
                let mut img_class = "image".to_string();
                match img.options.align {
                    crate::markdown::ImageAlign::Left => img_class.push_str(" align-left"),
                    crate::markdown::ImageAlign::Center => img_class.push_str(" align-center"),
                    crate::markdown::ImageAlign::Right => img_class.push_str(" align-right"),
                }
                if img.options.full_page {
                    img_class.push_str(" full-page");
                }

                let src = if let Some(path) = resolved_path {
                    let key = path.to_string_lossy().to_string();
                    let zip_img_path = if let Some(existing) = self.image_dest_map.get(&key) {
                        existing.clone()
                    } else {
                        let ext = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("png")
                            .to_ascii_lowercase();
                        let mime = mime_type_for_extension(&ext);
                        self.image_counter += 1;
                        let filename = format!("images/img_{:04}.{ext}", self.image_counter);

                        self.images.push(ImageAsset {
                            id: format!("img_{}", self.image_counter),
                            zip_path: filename.clone(),
                            source_path: path,
                            media_type: mime,
                            is_cover: false,
                        });

                        self.image_dest_map.insert(key, filename.clone());
                        filename
                    };
                    format!("../{}", zip_img_path)
                } else {
                    img.destination.clone()
                };

                let mut style_attr = String::new();
                if let Some(w) = img.options.width {
                    style_attr.push_str(&format!("width: {w}px; "));
                }
                if let Some(h) = img.options.height {
                    style_attr.push_str(&format!("height: {h}px; "));
                }

                let sec = self.ensure_section(&self.project.name);
                sec.html_body.push_str(&format!("<figure class=\"{img_class}\">\n"));
                if style_attr.is_empty() {
                    sec.html_body.push_str(&format!(
                        "  <img src=\"{}\" alt=\"{}\"/>\n",
                        escape_xml(&src),
                        escape_xml(&img.alt)
                    ));
                } else {
                    sec.html_body.push_str(&format!(
                        "  <img src=\"{}\" alt=\"{}\" style=\"{}\"/>\n",
                        escape_xml(&src),
                        escape_xml(&img.alt),
                        style_attr.trim()
                    ));
                }
                if !img.alt.is_empty() {
                    sec.html_body.push_str(&format!(
                        "  <figcaption>{}</figcaption>\n",
                        escape_xml(&img.alt)
                    ));
                }
                sec.html_body.push_str("</figure>\n");
            }
            Block::Rule => {
                let sec = self.ensure_section(&self.project.name);
                sec.html_body.push_str("<hr/>\n");
            }
            Block::Table(rows) => {
                let sec = self.ensure_section(&self.project.name);
                sec.html_body.push_str("<table>\n");
                for (r_ix, row) in rows.iter().enumerate() {
                    sec.html_body.push_str("<tr>\n");
                    let cell_tag = if r_ix == 0 { "th" } else { "td" };
                    for cell in row {
                        sec.html_body.push_str(&format!(
                            "  <{cell_tag}>{}</{cell_tag}>\n",
                            render_rich(cell)
                        ));
                    }
                    sec.html_body.push_str("</tr>\n");
                }
                sec.html_body.push_str("</table>\n");
            }
            Block::Footnotes(notes) => {
                let sec = self.ensure_section(&self.project.name);
                sec.html_body.push_str("<section class=\"footnotes\" epub:type=\"footnotes\">\n<ol class=\"footnotes-list\">\n");
                for (num, body) in notes {
                    let sec = self.ensure_section(&self.project.name);
                    sec.html_body.push_str(&format!("<li id=\"fn-{num}\">\n"));
                    for b in body {
                        self.render_block(b, sheet_dir);
                    }
                    let sec = self.ensure_section(&self.project.name);
                    sec.html_body.push_str("</li>\n");
                }
                let sec = self.ensure_section(&self.project.name);
                sec.html_body.push_str("</ol>\n</section>\n");
            }
        }
    }
}

/// 프로젝트 원고를 ePub 파일로 저장한다. 완성된 임시 파일로 목적 파일을 교체한다.
pub fn export_to_file(project_dir: &Path, output_path: &Path) -> Result<(), ExportError> {
    let project = load_project(project_dir)?;
    save_to_file(&project, output_path)
}

/// 선택한 그룹 또는 복수 시트의 접합 묶음만 ePub 파일로 저장한다.
pub fn export_selection_to_file(
    root: &Path,
    selected: &[PathBuf],
    output_path: &Path,
) -> Result<(), ExportError> {
    let project = super::load_selection(root, selected)?;
    save_to_file(&project, output_path)
}

fn save_to_file(project: &ExportProject, output_path: &Path) -> Result<(), ExportError> {
    let parent = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".writer-export-{}.tmp", ulid::Ulid::generate()));
    let file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result = (|| {
        let mut writer = BufWriter::new(file);
        build(project, &mut writer)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        std::fs::rename(&temporary, output_path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// 읽어 둔 프로젝트 원고를 ePub 아카이브로 만들어 스트림에 쓴다.
pub fn build<W: Write + Seek>(project: &ExportProject, writer: &mut W) -> Result<(), ExportError> {
    let mut zip = zip::ZipWriter::new(writer);

    // 1. mimetype: 반드시 첫 번째 파일이며 압축 없이(Stored) 저장되어야 함 (EPUB 규격 필수).
    let stored_options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    zip.start_file("mimetype", stored_options)?;
    zip.write_all(b"application/epub+zip")?;

    let deflated_options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    // 2. META-INF/container.xml
    zip.start_file("META-INF/container.xml", deflated_options)?;
    zip.write_all(container_xml().as_bytes())?;

    // 3. 생성 상태 초기화 및 표지 확인
    let mut builder = EpubBuilder::new(project);

    let cover_path = project.book.cover_path(&project.dir).filter(|p| p.is_file());
    let has_cover = if let Some(path) = cover_path {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_ascii_lowercase();
        let mime = mime_type_for_extension(&ext);
        let zip_path = format!("images/cover.{ext}");
        builder.images.push(ImageAsset {
            id: "cover-image".to_string(),
            zip_path: zip_path.clone(),
            source_path: path,
            media_type: mime,
            is_cover: true,
        });
        Some(zip_path)
    } else {
        None
    };

    // 4. 시트 원고를 순회하며 섹션 구성
    for sheet in &project.sheets {
        let sheet_dir = sheet.path.parent().unwrap_or(&project.dir);
        let blocks = crate::markdown::blocks(&sheet.content);

        // 앞 시트와 접합되지 않은 새 시트라면 이전 섹션을 마감하여 새 파일에서 시작
        if !sheet.glued {
            builder.finish_section();
        }

        for block in &blocks {
            builder.render_block(block, sheet_dir);
        }
    }

    builder.finish_section();

    // 최소 하나 이상의 섹션이 있도록 보장
    if builder.sections.is_empty() {
        builder.sections.push(Section {
            id: "sec_0001".to_string(),
            filename: "text/section_0001.xhtml".to_string(),
            title: project.book.title.clone(),
            html_body: "<p>내용이 비어 있습니다.</p>".to_string(),
        });
    }

    // 섹션 파일명과 ID를 1번부터 차례대로 정리
    for (ix, sec) in builder.sections.iter_mut().enumerate() {
        let num = ix + 1;
        let old_filename = sec.filename.clone();
        let new_filename = format!("text/section_{num:04}.xhtml");
        sec.id = format!("sec_{num:04}");
        sec.filename = new_filename.clone();
        for item in &mut builder.toc_items {
            if item.href.starts_with(&old_filename) {
                item.href = item.href.replace(&old_filename, &new_filename);
            }
        }
    }

    // 5. CSS 스타일 생성 및 저장
    let css_content = generate_css(project);
    zip.start_file("OEBPS/style.css", deflated_options)?;
    zip.write_all(css_content.as_bytes())?;

    // 6. 표지 XHTML 생성 (표지가 있을 경우)
    if let Some(ref cover_img_path) = has_cover {
        let cover_html = generate_cover_xhtml(cover_img_path);
        zip.start_file("OEBPS/cover.xhtml", deflated_options)?;
        zip.write_all(cover_html.as_bytes())?;
    }

    // 7. 각 섹션 XHTML 저장
    for sec in &builder.sections {
        let doc = wrap_xhtml(&sec.title, &sec.html_body);
        zip.start_file(format!("OEBPS/{}", sec.filename), deflated_options)?;
        zip.write_all(doc.as_bytes())?;
    }

    // 8. 이미지 에셋 저장
    for img in &builder.images {
        let mut image = File::open(&img.source_path)?;
        zip.start_file(format!("OEBPS/{}", img.zip_path), deflated_options)?;
        std::io::copy(&mut image, &mut zip)?;
    }

    // 9. 목차 생성 (toc.ncx)
    let toc_ncx = generate_toc_ncx(project, &builder.sections, &builder.toc_items);
    zip.start_file("OEBPS/toc.ncx", deflated_options)?;
    zip.write_all(toc_ncx.as_bytes())?;

    // 10. OEBPS/content.opf (패키지 문서)
    let content_opf = generate_content_opf(project, has_cover.is_some(), &builder.sections, &builder.images);
    zip.start_file("OEBPS/content.opf", deflated_options)?;
    zip.write_all(content_opf.as_bytes())?;

    zip.finish()?;
    Ok(())
}

fn container_xml() -> &'static str {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#
}

fn wrap_xhtml(title: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="ko">
<head>
  <meta charset="utf-8"/>
  <title>{}</title>
  <link rel="stylesheet" type="text/css" href="../style.css"/>
</head>
<body>
{}
</body>
</html>"#,
        escape_xml(title),
        body
    )
}

fn generate_cover_xhtml(cover_img_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="ko">
<head>
  <meta charset="utf-8"/>
  <title>표지</title>
  <link rel="stylesheet" type="text/css" href="style.css"/>
  <style type="text/css">
    body {{
      margin: 0;
      padding: 0;
      text-align: center;
      background-color: #ffffff;
    }}
    div.cover-wrapper {{
      height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
    }}
    img.cover-image {{
      max-width: 100%;
      max-height: 100%;
      object-fit: contain;
    }}
  </style>
</head>
<body>
  <div class="cover-wrapper">
    <img class="cover-image" src="{}" alt="표지"/>
  </div>
</body>
</html>"#,
        escape_xml(cover_img_path)
    )
}

/// 접두사(`prefix`)가 설정된 제목의 HTML 텍스트를 생성한다.
fn render_rich_with_prefix(text: &Rich, style: &ResolvedStyle, number: usize) -> String {
    let mut out = String::new();
    let prefix = style.prefix_text(number);
    if !prefix.is_empty() {
        // 접두사에 넣은 줄바꿈은 XHTML에서 줄바꿈이 되도록 <br/>로 바꾼다.
        out.push_str(&escape_xml(&prefix).replace('\n', "<br/>\n"));
    }
    out.push_str(&render_rich(text));
    out
}

/// 인라인 서식 텍스트를 XHTML로 변환한다.
fn render_rich(rich: &[Inline]) -> String {
    let mut out = String::new();
    for inline in rich {
        let escaped = escape_xml(&inline.text).replace('\n', "<br/>\n");
        let marks = inline.marks;
        if marks == Marks::default() {
            out.push_str(&escaped);
            continue;
        }

        let mut seg = escaped;
        if marks.code {
            seg = format!("<code>{seg}</code>");
        }
        if marks.strong {
            seg = format!("<strong>{seg}</strong>");
        }
        if marks.emphasis {
            seg = format!("<em>{seg}</em>");
        }
        if marks.strike {
            seg = format!("<del>{seg}</del>");
        }
        out.push_str(&seg);
    }
    out
}

/// 이미지 경로를 디스크 상의 실제 절대 경로로 찾는다.
fn resolve_image_path(sheet_dir: &Path, project_dir: &Path, destination: &str) -> Option<PathBuf> {
    if destination.starts_with("http://") || destination.starts_with("https://") {
        return None;
    }
    let clean = destination.strip_prefix("file://").unwrap_or(destination);
    let p = Path::new(clean);
    if p.is_absolute() && p.is_file() {
        return Some(p.to_path_buf());
    }

    // 1. 시트 폴더 기준 상대 경로
    let cand = sheet_dir.join(p);
    if cand.is_file() {
        return Some(cand);
    }

    // 2. 프로젝트 폴더 기준 상대 경로
    let cand = project_dir.join(p);
    if cand.is_file() {
        return Some(cand);
    }

    // 3. Extras/Attachments 폴더 기준
    let cand = project_dir.join("Extras").join("Attachments").join(p);
    if cand.is_file() {
        return Some(cand);
    }

    None
}

/// 프로젝트 책 스타일(`[styles.*]`)을 반영한 CSS를 생성한다.
fn generate_css(project: &ExportProject) -> String {
    let body_style = project.styles.resolve(StyleTarget::Body);
    let part_style = project.styles.resolve(StyleTarget::Part);
    let chapter_style = project.styles.resolve(StyleTarget::Chapter);
    let section_style = project.styles.resolve(StyleTarget::Section);

    let align_str = |align: Align| match align {
        Align::Left => "left",
        Align::Center => "center",
        Align::Right => "right",
        Align::Justify => "justify",
    };

    let font_decl = |font: &Option<String>| {
        font.as_deref()
            .map(|f| format!("font-family: \"{f}\", sans-serif;\n  "))
            .unwrap_or_default()
    };

    format!(
        r#"@charset "utf-8";

html, body {{
  margin: 0;
  padding: 0;
}}

body {{
  {body_font}font-size: {body_font_size}em;
  line-height: {body_line_height};
  text-align: {body_align};
  word-break: break-word;
}}

p {{
  margin-top: {body_space_before}em;
  margin-bottom: {body_space_after}em;
  text-indent: {body_indent}em;
}}

/* 별도 제목 페이지 (도비라) 컨테이너 */
.title-page {{
  box-sizing: border-box;
  min-height: 80vh;
  display: flex;
  flex-direction: column;
  justify-content: center;
  page-break-before: always;
  page-break-after: always;
  margin: 0;
  padding: 0;
}}

.title-page.vertical-top {{
  justify-content: flex-start;
  padding-top: 5em;
}}

.title-page.vertical-center {{
  justify-content: center;
}}

.title-page.vertical-bottom {{
  justify-content: flex-end;
  padding-bottom: 5em;
}}

/* 부 제목 (H1) */
h1.title-part, h1 {{
  {part_font}font-size: {part_font_size}em;
  font-weight: {part_font_weight};
  line-height: {part_line_height};
  text-align: {part_align};
  margin-top: {part_space_before}em;
  margin-bottom: {part_space_after}em;
  text-indent: 0;
}}

/* 장 제목 (H2) */
h2.title-chapter, h2 {{
  {chapter_font}font-size: {chapter_font_size}em;
  font-weight: {chapter_font_weight};
  line-height: {chapter_line_height};
  text-align: {chapter_align};
  margin-top: {chapter_space_before}em;
  margin-bottom: {chapter_space_after}em;
  text-indent: 0;
}}

/* 절 제목 (H3) */
h3.title-section, h3 {{
  {section_font}font-size: {section_font_size}em;
  font-weight: {section_font_weight};
  line-height: {section_line_height};
  text-align: {section_align};
  margin-top: {section_space_before}em;
  margin-bottom: {section_space_after}em;
  text-indent: 0;
}}

h4, h5, h6 {{
  margin-top: 1.2em;
  margin-bottom: 0.4em;
  text-indent: 0;
}}

blockquote {{
  margin: 1.2em 1.5em;
  padding-left: 1em;
  border-left: 3px solid #ccc;
  color: #555;
}}

pre, code {{
  font-family: monospace, monospace;
  font-size: 0.9em;
  background-color: #f7f7f7;
}}

pre {{
  padding: 0.8em;
  overflow-x: auto;
  border-radius: 4px;
}}

code {{
  padding: 0.1em 0.3em;
  border-radius: 3px;
}}

figure.image {{
  margin: 1.5em 0;
  text-align: center;
}}

figure.image img {{
  max-width: 100%;
  height: auto;
}}

figure.image figcaption {{
  font-size: 0.85em;
  color: #666;
  margin-top: 0.4em;
}}

figure.align-left {{
  text-align: left;
}}

figure.align-center {{
  text-align: center;
}}

figure.align-right {{
  text-align: right;
}}

figure.full-page {{
  height: 95vh;
  display: flex;
  align-items: center;
  justify-content: center;
}}

hr {{
  border: 0;
  border-top: 1px solid #ddd;
  margin: 2em 0;
}}

table {{
  border-collapse: collapse;
  width: 100%;
  margin: 1.5em 0;
}}

th, td {{
  border: 1px solid #ddd;
  padding: 0.5em 0.8em;
  text-align: left;
}}

th {{
  background-color: #f5f5f5;
  font-weight: bold;
}}

section.footnotes {{
  margin-top: 3em;
  padding-top: 1em;
  border-top: 1px solid #eee;
  font-size: 0.85em;
}}
"#,
        body_font = font_decl(&body_style.font),
        body_font_size = body_style.font_size,
        body_line_height = body_style.line_height,
        body_align = align_str(body_style.align),
        body_indent = body_style.first_line_indent,
        body_space_before = body_style.space_before,
        body_space_after = body_style.space_after,

        part_font = font_decl(&part_style.font),
        part_font_size = part_style.font_size * 1.6,
        part_font_weight = part_style.font_weight,
        part_line_height = part_style.line_height,
        part_align = align_str(part_style.align),
        part_space_before = part_style.space_before,
        part_space_after = part_style.space_after,

        chapter_font = font_decl(&chapter_style.font),
        chapter_font_size = chapter_style.font_size * 1.3,
        chapter_font_weight = chapter_style.font_weight,
        chapter_line_height = chapter_style.line_height,
        chapter_align = align_str(chapter_style.align),
        chapter_space_before = chapter_style.space_before,
        chapter_space_after = chapter_style.space_after,

        section_font = font_decl(&section_style.font),
        section_font_size = section_style.font_size * 1.1,
        section_font_weight = section_style.font_weight,
        section_line_height = section_style.line_height,
        section_align = align_str(section_style.align),
        section_space_before = section_style.space_before,
        section_space_after = section_style.space_after,
    )
}

/// EPUB NCX (`toc.ncx`) 생성.
fn generate_toc_ncx(project: &ExportProject, sections: &[Section], toc_items: &[TocItem]) -> String {
    let title = if project.book.title.is_empty() {
        &project.name
    } else {
        &project.book.title
    };
    let identifier = if project.book.identifier.is_empty() {
        "urn:uuid:writer-book"
    } else {
        &project.book.identifier
    };

    let mut nav_points = String::new();
    let mut play_order = 1;

    if toc_items.is_empty() {
        for sec in sections {
            nav_points.push_str(&format!(
                r#"    <navPoint id="navPoint-{play_order}" playOrder="{play_order}">
      <navLabel><text>{}</text></navLabel>
      <content src="{}"/>
    </navPoint>
"#,
                escape_xml(&toc_label(&sec.title)),
                escape_xml(&sec.filename)
            ));
            play_order += 1;
        }
    } else {
        for item in toc_items {
            nav_points.push_str(&format!(
                r#"    <navPoint id="navPoint-{}-{play_order}" playOrder="{play_order}">
      <navLabel><text>{}</text></navLabel>
      <content src="{}"/>
    </navPoint>
"#,
                escape_xml(&item.id),
                escape_xml(&toc_label(&item.title)),
                escape_xml(&item.href)
            ));
            play_order += 1;
        }
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head>
    <meta name="dtb:uid" content="{}"/>
    <meta name="dtb:depth" content="2"/>
    <meta name="dtb:totalPageCount" content="0"/>
    <meta name="dtb:maxPageNumber" content="0"/>
  </head>
  <docTitle>
    <text>{}</text>
  </docTitle>
  <navMap>
{nav_points}  </navMap>
</ncx>"#,
        escape_xml(identifier),
        escape_xml(title)
    )
}

/// `OEBPS/content.opf` (패키지 문서) 생성.
fn generate_content_opf(
    project: &ExportProject,
    has_cover: bool,
    sections: &[Section],
    images: &[ImageAsset],
) -> String {
    let title = if project.book.title.is_empty() {
        &project.name
    } else {
        &project.book.title
    };
    let lang = if project.book.language.is_empty() {
        "ko"
    } else {
        &project.book.language
    };
    let identifier = if project.book.identifier.is_empty() {
        "urn:uuid:writer-book"
    } else {
        &project.book.identifier
    };

    let mut creators = String::new();
    for (i, author) in project.book.authors.iter().enumerate() {
        creators.push_str(&format!(
            "    <dc:creator id=\"creator_{i}\">{}</dc:creator>\n",
            escape_xml(author)
        ));
    }
    if creators.is_empty() {
        creators.push_str("    <dc:creator id=\"creator_0\">작가 미상</dc:creator>\n");
    }

    let description_tag = if !project.book.description.is_empty() {
        format!(
            "    <dc:description>{}</dc:description>\n",
            escape_xml(&project.book.description)
        )
    } else {
        String::new()
    };

    let cover_meta = if has_cover {
        "    <meta name=\"cover\" content=\"cover-image\"/>\n"
    } else {
        ""
    };

    // manifest 생성
    let mut manifest = String::new();
    manifest.push_str("    <item id=\"style\" href=\"style.css\" media-type=\"text/css\"/>\n");
    manifest.push_str("    <item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>\n");

    if has_cover {
        manifest.push_str("    <item id=\"cover\" href=\"cover.xhtml\" media-type=\"application/xhtml+xml\"/>\n");
    }

    for sec in sections {
        manifest.push_str(&format!(
            "    <item id=\"{}\" href=\"{}\" media-type=\"application/xhtml+xml\"/>\n",
            escape_xml(&sec.id),
            escape_xml(&sec.filename)
        ));
    }

    for img in images {
        let prop = if img.is_cover {
            " properties=\"cover-image\""
        } else {
            ""
        };
        manifest.push_str(&format!(
            "    <item id=\"{}\" href=\"{}\" media-type=\"{}\"{prop}/>\n",
            escape_xml(&img.id),
            escape_xml(&img.zip_path),
            img.media_type
        ));
    }

    // spine 생성
    let mut spine = String::new();
    if has_cover {
        spine.push_str("    <itemref idref=\"cover\"/>\n");
    }
    for sec in sections {
        spine.push_str(&format!("    <itemref idref=\"{}\"/>\n", escape_xml(&sec.id)));
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="BookId" xml:lang="{lang}">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="BookId">{}</dc:identifier>
    <dc:title>{}</dc:title>
    <dc:language>{}</dc:language>
{creators}{description_tag}    <meta property="dcterms:modified">2026-10-21T00:00:00Z</meta>
{cover_meta}  </metadata>
  <manifest>
{manifest}  </manifest>
  <spine toc="ncx">
{spine}  </spine>
</package>"#,
        escape_xml(identifier),
        escape_xml(title),
        escape_xml(lang)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::ExportSheet;
    use crate::library::{BookMeta, Styles};
    use std::io::Cursor;

    #[test]
    fn epub_builds_valid_archive() {
        let mut styles = Styles::default();
        styles.chapter.prefix = Some("제{n}장 ".to_string());

        let project = ExportProject {
            name: "테스트 프로젝트".to_string(),
            dir: PathBuf::from("fake/project"),
            book: BookMeta {
                title: "나의 소설".to_string(),
                authors: vec!["김작가".to_string()],
                language: "ko".to_string(),
                description: "재미있는 이야기".to_string(),
                cover: None,
                identifier: "urn:uuid:12345678-1234-1234-1234-123456789abc".to_string(),
            },
            styles,
            sheets: vec![
                ExportSheet {
                    path: PathBuf::from("fake/project/01.md"),
                    glued: false,
                    content: "# 1부 시작\n\n부 제목은 기본이 별도 제목 페이지(도비라)다.\n".to_string(),
                },
                ExportSheet {
                    path: PathBuf::from("fake/project/02.md"),
                    glued: true,
                    content: "## 첫 이야기\n\n이 문장은 앞 시트에 이어진 본문이다.\n".to_string(),
                },
                ExportSheet {
                    path: PathBuf::from("fake/project/03.md"),
                    glued: false,
                    content: "## 두 번째 이야기\n\n모험을 떠났습니다.\n".to_string(),
                },
            ],
        };

        let mut buffer = Cursor::new(Vec::new());
        build(&project, &mut buffer).unwrap();

        let bytes = buffer.into_inner();
        assert!(!bytes.is_empty());

        let mut reader = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();

        // 1. mimetype 확인
        {
            let mut mimetype = reader.by_name("mimetype").unwrap();
            let mut mime_str = String::new();
            std::io::Read::read_to_string(&mut mimetype, &mut mime_str).unwrap();
            assert_eq!(mime_str, "application/epub+zip");
            assert_eq!(mimetype.compression(), CompressionMethod::Stored);
        }

        // 2. 주요 파일 존재 확인 (별도 nav 페이지 없이 toc.ncx를 목차로 사용)
        assert!(reader.by_name("META-INF/container.xml").is_ok());
        assert!(reader.by_name("OEBPS/content.opf").is_ok());
        assert!(reader.by_name("OEBPS/style.css").is_ok());
        assert!(reader.by_name("OEBPS/toc.ncx").is_ok());
        assert!(reader.by_name("OEBPS/nav.xhtml").is_err());

        // 3. 섹션 파일 구조 검증:
        // - 01.md의 "# 1부 시작"은 Placement::Page 이므로 독립된 title-page 섹션 생성
        // - 그 뒤의 "부 제목은..." 본문은 다음 섹션
        // - "## 첫 이야기"는 page_break 기본값에 따라 섹션 분할
        assert!(reader.by_name("OEBPS/text/section_0001.xhtml").is_ok());
        assert!(reader.by_name("OEBPS/text/section_0002.xhtml").is_ok());

        // 4. 별도 제목 페이지(title-page) 및 접두사 확인
        let mut title_page = reader.by_name("OEBPS/text/section_0001.xhtml").unwrap();
        let mut title_content = String::new();
        std::io::Read::read_to_string(&mut title_page, &mut title_content).unwrap();
        assert!(title_content.contains("class=\"title-page vertical-center\""));
        assert!(title_content.contains("class=\"title-part\""));
        assert!(title_content.contains("1부 시작"));

        // 5. 목차(toc.ncx) 접두사("제1장 첫 이야기") 반영 확인
        drop(title_page);
        let mut ncx_file = reader.by_name("OEBPS/toc.ncx").unwrap();
        let mut ncx_content = String::new();
        std::io::Read::read_to_string(&mut ncx_file, &mut ncx_content).unwrap();
        assert!(ncx_content.contains("제1장 첫 이야기"));
    }

    #[test]
    fn prefix_newline_becomes_line_break() {
        let mut style = Styles::default().resolve(StyleTarget::Chapter);
        style.prefix = "제{n}장\n".to_string();
        let html = render_rich_with_prefix(&Rich::new(), &style, 2);
        assert_eq!(html, "제2장<br/>\n");
    }

    #[test]
    fn toc_flattens_prefix_newline() {
        let mut styles = Styles::default();
        styles.chapter.prefix = Some("제{n}장\n".to_string());

        let project = ExportProject {
            name: "테스트".to_string(),
            dir: PathBuf::from("fake/project"),
            book: BookMeta::default(),
            styles,
            sheets: vec![ExportSheet {
                path: PathBuf::from("fake/project/01.md"),
                glued: false,
                content: "## 첫 이야기\n\n본문.\n".to_string(),
            }],
        };

        let mut buffer = Cursor::new(Vec::new());
        build(&project, &mut buffer).unwrap();

        let mut reader = zip::ZipArchive::new(Cursor::new(buffer.into_inner())).unwrap();
        let mut ncx = reader.by_name("OEBPS/toc.ncx").unwrap();
        let mut content = String::new();
        std::io::Read::read_to_string(&mut ncx, &mut content).unwrap();
        assert!(content.contains("<text>제1장 첫 이야기</text>"), "{content}");
    }

    #[test]
    fn toc_flattens_title_break() {
        let project = ExportProject {
            name: "테스트".to_string(),
            dir: PathBuf::from("fake/project"),
            book: BookMeta::default(),
            styles: Styles::default(),
            sheets: vec![ExportSheet {
                path: PathBuf::from("fake/project/01.md"),
                glued: false,
                content: "## 제목<br>부제\n\n본문.\n".to_string(),
            }],
        };

        let mut buffer = Cursor::new(Vec::new());
        build(&project, &mut buffer).unwrap();

        let mut reader = zip::ZipArchive::new(Cursor::new(buffer.into_inner())).unwrap();
        let mut ncx = reader.by_name("OEBPS/toc.ncx").unwrap();
        let mut content = String::new();
        std::io::Read::read_to_string(&mut ncx, &mut content).unwrap();
        assert!(content.contains("<text>제목 부제</text>"), "{content}");
        assert!(!content.contains("제목\n부제"), "{content}");
    }

    #[test]
    fn prefix_uses_roman_numbering() {
        let mut styles = Styles::default();
        styles.chapter.prefix = Some("제{n}장 ".to_string());
        styles.chapter.numbering = Some(crate::library::Numbering::Roman);
        let style = styles.resolve(StyleTarget::Chapter);
        let html = render_rich_with_prefix(&Rich::new(), &style, 4);
        assert_eq!(html, "제IV장 ");
    }
}
