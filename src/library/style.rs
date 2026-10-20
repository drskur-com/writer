//! 책 스타일(본문·부/장/절 제목): 프로젝트 `.writer.toml`의 `[styles.<대상>]` 표(ADR-0009).
//!
//! 에디터 렌더링과 내보내기가 같은 값을 쓴다. 파일에는 사용자가 바꾼 값만 쓰고,
//! 빠진 값은 대상별 기본값([`StyleTarget::defaults`])으로 채운다.
//! 다른 프로젝트의 스타일은 값을 그대로 복사해 가져온다([`import_styles`]).

use std::io;
use std::path::Path;

use super::GROUP_META_FILE;
use super::meta::write_meta_values;

/// 스타일 대상. 파일 키(`key`)는 바꾸지 않는다.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StyleTarget {
    /// 본문 문단.
    Body,
    /// 부 제목(`#`).
    Part,
    /// 장 제목(`##`).
    Chapter,
    /// 절 제목(`###`).
    Section,
}

impl StyleTarget {
    /// 화면에 늘어놓는 순서.
    pub const ALL: [StyleTarget; 4] = [
        StyleTarget::Body,
        StyleTarget::Part,
        StyleTarget::Chapter,
        StyleTarget::Section,
    ];

    /// 파일 키(`[styles.<key>]`).
    pub fn key(self) -> &'static str {
        match self {
            StyleTarget::Body => "body",
            StyleTarget::Part => "title:part",
            StyleTarget::Chapter => "title:chapter",
            StyleTarget::Section => "title:section",
        }
    }

    /// 표시 이름.
    pub fn title(self) -> &'static str {
        match self {
            StyleTarget::Body => "본문",
            StyleTarget::Part => "부 제목",
            StyleTarget::Chapter => "장 제목",
            StyleTarget::Section => "절 제목",
        }
    }

    /// 제목 대상인지.
    pub fn is_title(self) -> bool {
        self != StyleTarget::Body
    }

    /// ATX 제목 레벨이 쓰는 대상. `#`~`###`만 제목 대상이고 나머지는 없다.
    pub fn for_heading(level: u8) -> Option<StyleTarget> {
        match level {
            1 => Some(StyleTarget::Part),
            2 => Some(StyleTarget::Chapter),
            3 => Some(StyleTarget::Section),
            _ => None,
        }
    }

    /// 대상의 기본 스타일. 파일에 없는 값은 이것으로 채운다.
    pub fn defaults(self) -> ResolvedStyle {
        let body = ResolvedStyle {
            font: None,
            embed_font: false,
            font_size: 1.0,
            line_height: 1.6,
            align: Align::Justify,
            first_line_indent: 1.0,
            space_before: 0.0,
            space_after: 0.0,
        };
        let title = |font_size, align, space_before, space_after| ResolvedStyle {
            font_size,
            line_height: 1.3,
            align,
            first_line_indent: 0.0,
            space_before,
            space_after,
            ..body.clone()
        };
        match self {
            StyleTarget::Body => body,
            StyleTarget::Part => title(1.6, Align::Center, 4.0, 2.0),
            StyleTarget::Chapter => title(1.4, Align::Center, 3.0, 1.5),
            StyleTarget::Section => title(1.2, Align::Left, 1.5, 0.75),
        }
    }
}

/// 문단 정렬.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

impl Align {
    pub const ALL: [Align; 4] = [Align::Left, Align::Center, Align::Right, Align::Justify];

    /// 표시 이름.
    pub fn title(self) -> &'static str {
        match self {
            Align::Left => "왼쪽",
            Align::Center => "가운데",
            Align::Right => "오른쪽",
            Align::Justify => "양쪽",
        }
    }
}

/// 한 대상의 스타일. 비어 있는 값은 기본값을 따른다. 길이 단위는 em(본문 글자 크기 대비)이다.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct StyleSpec {
    /// 서체 이름(에디터 서체). 없으면 앱 설정의 에디터 서체. 제목은 없으면 본문 서체를 따른다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// 내보낼 때 서체 파일을 넣는지. 넣으면 책도 에디터와 같은 서체, 아니면 리더 기본 서체로 보인다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_font: Option<bool>,
    /// 글자 크기(본문 대비 배율).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// 줄 간격(글자 크기 대비).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<Align>,
    /// 첫 줄 들여쓰기(em).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_line_indent: Option<f64>,
    /// 앞 여백(em).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_before: Option<f64>,
    /// 뒤 여백(em). 본문에서는 문단 사이 간격이다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_after: Option<f64>,
}

impl StyleSpec {
    /// 바꾼 값이 하나도 없는지.
    pub fn is_empty(&self) -> bool {
        *self == StyleSpec::default()
    }

    /// 빠진 값을 대상 기본값으로 채운다.
    pub fn resolve(&self, target: StyleTarget) -> ResolvedStyle {
        let defaults = target.defaults();
        ResolvedStyle {
            font: self.font.clone().or(defaults.font),
            embed_font: self.embed_font.unwrap_or(defaults.embed_font),
            font_size: self.font_size.unwrap_or(defaults.font_size),
            line_height: self.line_height.unwrap_or(defaults.line_height),
            align: self.align.unwrap_or(defaults.align),
            first_line_indent: self.first_line_indent.unwrap_or(defaults.first_line_indent),
            space_before: self.space_before.unwrap_or(defaults.space_before),
            space_after: self.space_after.unwrap_or(defaults.space_after),
        }
    }

    /// 파일에 쓰기 좋게 숫자를 소수 둘째 자리로 맞춘다(슬라이더가 만드는 1.6500001 같은 값).
    fn rounded(&self) -> StyleSpec {
        let round = |value: Option<f64>| value.map(|value| (value * 100.0).round() / 100.0);
        StyleSpec {
            font_size: round(self.font_size),
            line_height: round(self.line_height),
            first_line_indent: round(self.first_line_indent),
            space_before: round(self.space_before),
            space_after: round(self.space_after),
            ..self.clone()
        }
    }
}

/// 기본값까지 채운 스타일.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStyle {
    pub font: Option<String>,
    pub embed_font: bool,
    pub font_size: f64,
    pub line_height: f64,
    pub align: Align,
    pub first_line_indent: f64,
    pub space_before: f64,
    pub space_after: f64,
}

/// 프로젝트의 책 스타일(`[styles]` 표). 바꾸지 않은 대상은 파일에 쓰지 않는다.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct Styles {
    #[serde(skip_serializing_if = "StyleSpec::is_empty")]
    pub body: StyleSpec,
    #[serde(rename = "title:part", skip_serializing_if = "StyleSpec::is_empty")]
    pub part: StyleSpec,
    #[serde(rename = "title:chapter", skip_serializing_if = "StyleSpec::is_empty")]
    pub chapter: StyleSpec,
    #[serde(rename = "title:section", skip_serializing_if = "StyleSpec::is_empty")]
    pub section: StyleSpec,
}

impl Styles {
    pub fn get(&self, target: StyleTarget) -> &StyleSpec {
        match target {
            StyleTarget::Body => &self.body,
            StyleTarget::Part => &self.part,
            StyleTarget::Chapter => &self.chapter,
            StyleTarget::Section => &self.section,
        }
    }

    pub fn get_mut(&mut self, target: StyleTarget) -> &mut StyleSpec {
        match target {
            StyleTarget::Body => &mut self.body,
            StyleTarget::Part => &mut self.part,
            StyleTarget::Chapter => &mut self.chapter,
            StyleTarget::Section => &mut self.section,
        }
    }

    /// 대상의 스타일을 기본값까지 채워 돌려준다. 제목의 서체·서체 포함은 정하지 않으면 본문을 따른다.
    pub fn resolve(&self, target: StyleTarget) -> ResolvedStyle {
        let spec = self.get(target);
        if !target.is_title() {
            return spec.resolve(target);
        }
        let inherited = StyleSpec {
            font: spec.font.clone().or_else(|| self.body.font.clone()),
            embed_font: spec.embed_font.or(self.body.embed_font),
            ..spec.clone()
        };
        inherited.resolve(target)
    }

    /// 바꾼 값이 하나도 없는지.
    pub fn is_empty(&self) -> bool {
        *self == Styles::default()
    }
}

/// 프로젝트의 책 스타일을 읽는다. 없거나 깨지면 기본값.
pub fn read_styles(project_dir: &Path) -> Styles {
    std::fs::read_to_string(project_dir.join(GROUP_META_FILE))
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|mut table| table.remove("styles"))
        .and_then(|styles| styles.try_into().ok())
        .unwrap_or_default()
}

/// 책 스타일을 `.writer.toml`의 `[styles]`에 쓴다. 다른 키는 그대로 두고, 비면 표를 지운다.
pub fn write_styles(project_dir: &Path, styles: &Styles) -> io::Result<()> {
    let styles = Styles {
        body: styles.body.rounded(),
        part: styles.part.rounded(),
        chapter: styles.chapter.rounded(),
        section: styles.section.rounded(),
    };
    let value = if styles.is_empty() {
        None
    } else {
        Some(
            toml::Value::try_from(&styles)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?,
        )
    };
    write_meta_values(project_dir, [("styles", value)])
}

/// 다른 프로젝트의 책 스타일을 이 프로젝트에 그대로 복사하고, 복사한 스타일을 돌려준다.
pub fn import_styles(source_dir: &Path, project_dir: &Path) -> io::Result<Styles> {
    let styles = read_styles(source_dir);
    write_styles(project_dir, &styles)?;
    Ok(styles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::test_util::temp_dir;
    use crate::library::*;

    #[test]
    fn styles_round_trip_and_keep_other_meta() {
        let root = temp_dir("styles");
        initialize(&root).unwrap();
        let projects = root.join(SectionKind::Projects.dir());
        let dir = create_project(&projects, "소설", Some("book"), None).unwrap();
        assert!(read_styles(&dir).is_empty());

        let mut styles = Styles::default();
        styles.body.font = Some("Noto Serif KR".into());
        styles.body.line_height = Some(1.6500001);
        styles.chapter.align = Some(Align::Left);
        write_styles(&dir, &styles).unwrap();

        let text = std::fs::read_to_string(dir.join(GROUP_META_FILE)).unwrap();
        assert!(text.contains("[styles.body]"), "{text}");
        assert!(text.contains("[styles.\"title:chapter\"]"), "{text}");
        assert!(!text.contains("title:part"), "{text}");
        assert!(text.contains("line_height = 1.65"), "{text}");

        let read = read_styles(&dir);
        assert_eq!(read.body.font.as_deref(), Some("Noto Serif KR"));
        assert_eq!(read.body.line_height, Some(1.65));
        assert_eq!(read.resolve(StyleTarget::Chapter).align, Align::Left);
        // 쓰지 않은 값은 기본값.
        assert_eq!(read.resolve(StyleTarget::Chapter).font_size, 1.4);
        let project = scan(&root).unwrap().find_project(&dir).cloned().unwrap();
        assert_eq!(project.icon.as_deref(), Some("book"));

        // 모두 기본값으로 돌리면 표가 사라진다.
        write_styles(&dir, &Styles::default()).unwrap();
        let text = std::fs::read_to_string(dir.join(GROUP_META_FILE)).unwrap();
        assert!(!text.contains("styles"), "{text}");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn import_copies_styles_from_other_project() {
        let root = temp_dir("styles-import");
        initialize(&root).unwrap();
        let projects = root.join(SectionKind::Projects.dir());
        let source = create_project(&projects, "원본", None, None).unwrap();
        let target = create_project(&projects, "새 책", None, None).unwrap();

        let mut styles = Styles::default();
        styles.part.font_size = Some(2.0);
        write_styles(&source, &styles).unwrap();
        let mut own = Styles::default();
        own.body.align = Some(Align::Left);
        write_styles(&target, &own).unwrap();

        assert_eq!(import_styles(&source, &target).unwrap(), styles);
        // 덮어써서 원래 값은 남지 않는다.
        assert_eq!(read_styles(&target), styles);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn titles_inherit_body_font_and_embedding() {
        let mut styles = Styles::default();
        styles.body.font = Some("Noto Serif KR".into());
        styles.body.embed_font = Some(true);
        let chapter = styles.resolve(StyleTarget::Chapter);
        assert_eq!(chapter.font.as_deref(), Some("Noto Serif KR"));
        assert!(chapter.embed_font);

        styles.chapter.font = Some("Pretendard".into());
        styles.chapter.embed_font = Some(false);
        let chapter = styles.resolve(StyleTarget::Chapter);
        assert_eq!(chapter.font.as_deref(), Some("Pretendard"));
        assert!(!chapter.embed_font);
    }

    #[test]
    fn heading_levels_map_to_title_targets() {
        assert_eq!(StyleTarget::for_heading(1), Some(StyleTarget::Part));
        assert_eq!(StyleTarget::for_heading(2), Some(StyleTarget::Chapter));
        assert_eq!(StyleTarget::for_heading(3), Some(StyleTarget::Section));
        assert_eq!(StyleTarget::for_heading(4), None);
    }
}
