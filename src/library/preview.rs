//! 시트 본문 앞부분에서 목록용 제목·발췌를 뽑는다.

use std::path::Path;

/// 시트 본문에서 얻은 목록용 미리보기.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    /// 표시 제목: 본문 첫 줄(헤딩 표시 제거). 빈 시트는 빈 문자열.
    pub title: String,
    /// 첫 줄의 ATX 헤딩 레벨(1~6). 헤딩이 아니면 0.
    pub level: u8,
    /// 목록 카드에 보일 발췌: 제목 다음 줄들을 한 줄로 이은 것.
    pub excerpt: String,
}

impl Preview {
    /// 본문이 비어 있는지(공백·frontmatter뿐이어도 빈 것). 목록에서 "빈 시트"로 표시한다.
    pub fn is_empty(&self) -> bool {
        self.title.is_empty()
    }
}

/// 제목·발췌를 얻으려고 읽는 파일 앞부분 크기.
const PREVIEW_BYTES: u64 = 4096;

/// 발췌 최대 글자 수.
const EXCERPT_CHARS: usize = 200;

/// 시트 파일 앞부분을 읽어 미리보기를 만든다. 읽지 못하면 빈 미리보기.
pub fn read_preview(path: &Path) -> Preview {
    preview_of(&read_head(path))
}

/// 파일 앞부분(`PREVIEW_BYTES`)을 UTF-8로 읽는다. 끝에서 잘린 글자는 버린다.
fn read_head(path: &Path) -> String {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.take(PREVIEW_BYTES).read_to_end(&mut bytes);
    }
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => {
            let valid = err.utf8_error().valid_up_to();
            let mut bytes = err.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).unwrap_or_default()
        }
    }
}

/// 본문에서 제목(첫 줄)과 그 헤딩 레벨, 발췌(그다음 줄들)를 뽑는다.
///
/// BOM과 앞쪽 YAML frontmatter(`---` … `---`/`...`)는 건너뛰고,
/// 각 줄의 ATX 헤딩 표시(`# `, 끝의 `#`)는 지운다. 빈 줄은 무시한다.
fn preview_of(text: &str) -> Preview {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines().peekable();

    if lines.peek().is_some_and(|line| line.trim_end() == "---") {
        lines.next();
        for line in lines.by_ref() {
            if matches!(line.trim_end(), "---" | "...") {
                break;
            }
        }
    }

    let mut content = lines.filter(|line| !strip_heading(line).is_empty());
    let first = content.next().unwrap_or_default();
    // 카드 제목에서는 첫 줄의 `{unnumbered}` 속성(ADR-0018)도 지운다.
    let title = strip_unnumbered(&strip_breaks(strip_heading(first))).to_string();
    let level = heading_level(first);
    let content = content.map(strip_heading);

    let mut excerpt = String::new();
    for line in content {
        if !excerpt.is_empty() {
            excerpt.push(' ');
        }
        excerpt.push_str(line);
        if excerpt.chars().count() >= EXCERPT_CHARS {
            break;
        }
    }
    let excerpt: String = excerpt.chars().take(EXCERPT_CHARS).collect();
    Preview {
        title,
        level,
        excerpt: strip_breaks(&excerpt),
    }
}

/// 인라인 줄바꿈 태그(`<br>`)는 목록 한 줄에 맞게 스페이스로 바꾼다.
fn strip_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(ix) = rest.find('<') {
        let (head, tail) = rest.split_at(ix);
        out.push_str(head);
        let Some(end) = tail.find('>').map(|end| end + 1) else {
            out.push_str(tail);
            return out;
        };
        let tag = &tail[..end];
        if is_break_tag(tag) {
            out.push(' ');
        } else {
            out.push_str(tag);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// 줄바꿈 태그인지(`<br>`, `<br/>`, `<br />`, 대소문자 무시).
fn is_break_tag(tag: &str) -> bool {
    matches!(
        tag.trim().to_ascii_lowercase().as_str(),
        "<br>" | "<br/>" | "<br />"
    )
}

/// 앞뒤 공백과 ATX 헤딩 표시를 지운다. `#태그`처럼 뒤에 공백이 없으면 헤딩이 아니다.
fn strip_heading(line: &str) -> &str {
    let line = line.trim();
    match heading_level(line) {
        0 => line,
        level => line[usize::from(level)..]
            .trim()
            .trim_end_matches('#')
            .trim_end(),
    }
}

/// 끝의 `{unnumbered}` 속성과 그 앞 공백을 지운다.
fn strip_unnumbered(title: &str) -> &str {
    let title = title.trim_end();
    match title.strip_suffix(crate::markdown::UNNUMBERED) {
        Some(head) => head.trim_end(),
        None => title,
    }
}

/// 줄의 ATX 헤딩 레벨(1~6). 헤딩이 아니면 0.
fn heading_level(line: &str) -> u8 {
    let line = line.trim();
    let hashes = line.chars().take_while(|&c| c == '#').count();
    let rest = &line[hashes.min(line.len())..];
    if (1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with([' ', '\t'])) {
        hashes as u8
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title_and_excerpt(text: &str) -> (String, String) {
        let preview = preview_of(text);
        (preview.title, preview.excerpt)
    }

    #[test]
    fn title_level_is_first_line_heading() {
        assert_eq!(preview_of("# 하나\n본문").level, 1);
        assert_eq!(preview_of("\n### 셋 ###\n").level, 3);
        // 빈 헤딩 줄은 건너뛰고 다음 줄을 본다.
        assert_eq!(preview_of("##\n그냥 글").level, 0);
        assert_eq!(preview_of("#태그").level, 0);
        assert_eq!(preview_of("####### 일곱").level, 0);
    }

    #[test]
    fn title_strips_unnumbered_attribute() {
        assert_eq!(preview_of("## 서장 {unnumbered}\n본문").title, "서장");
        assert_eq!(preview_of("## 서장{unnumbered}\n본문").title, "서장");
        // 본문 줄의 속성은 발췌에 그대로 남는다.
        assert_eq!(
            title_and_excerpt("# 제목\n## 끝 {unnumbered}\n"),
            ("제목".to_string(), "끝 {unnumbered}".to_string())
        );
    }

    #[test]
    fn title_and_excerpt_from_content() {
        // 헤딩 표시 제거, 빈 줄 무시, 다음 줄들은 발췌로
        assert_eq!(
            title_and_excerpt("# 제목 #\n\n첫 문단\n둘째 줄\n"),
            ("제목".to_string(), "첫 문단 둘째 줄".to_string())
        );
        // 헤딩이 아니어도 첫 줄이 제목, `#태그`는 헤딩이 아니다
        assert_eq!(
            title_and_excerpt("  그냥 메모\n#태그"),
            ("그냥 메모".to_string(), "#태그".to_string())
        );
        // frontmatter와 BOM은 건너뛴다
        assert_eq!(
            title_and_excerpt("\u{feff}---\nkeywords: [a]\n---\n## 본문\n"),
            ("본문".to_string(), String::new())
        );
        // 공백·frontmatter뿐이면 빈 시트
        assert_eq!(title_and_excerpt(" \n\n"), (String::new(), String::new()));
        assert_eq!(
            title_and_excerpt("---\ngoal: 1\n---\n"),
            (String::new(), String::new())
        );
        // 제목의 줄바꿈 태그(`<br>`)는 스페이스로 바꾼다
        assert_eq!(
            title_and_excerpt("# 제목<br>부제\n\n본문\n"),
            ("제목 부제".to_string(), "본문".to_string())
        );
        assert_eq!(
            title_and_excerpt("# 제목<BR />부제"),
            ("제목 부제".to_string(), String::new())
        );
        // 발췌는 EXCERPT_CHARS에서 자른다
        let long = format!("t\n{}", "가".repeat(500));
        assert_eq!(title_and_excerpt(&long).1.chars().count(), EXCERPT_CHARS);
    }
}
