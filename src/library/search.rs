//! 라이브러리 시트 본문 검색. ripgrep의 검색 엔진(`grep` 크레이트: grep-searcher + grep-regex)으로
//! 시트 파일을 줄 단위로 훑는다. 색인은 두지 않는다.

use std::ops::Range;
use std::path::{Path, PathBuf};

use grep::matcher::Matcher as _;
use grep::regex::{RegexMatcher, RegexMatcherBuilder};
use grep::searcher::sinks::Lossy;
use grep::searcher::{BinaryDetection, SearcherBuilder};

/// 발췌에서 첫 일치 앞에 남기는 글자 수.
const SNIPPET_BEFORE: usize = 30;

/// 발췌 최대 글자 수.
const SNIPPET_CHARS: usize = 160;

/// 검색어가 든 시트 하나.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub path: PathBuf,
    /// 검색어가 든 줄 수.
    pub lines: usize,
    /// 첫 일치 줄의 발췌(앞뒤를 줄였으면 `…`).
    pub snippet: String,
    /// 발췌 안 일치 위치(바이트).
    pub ranges: Vec<Range<usize>>,
}

/// 검색어 매처. 글자 그대로 찾고, 검색어에 대문자가 없으면 대소문자를 가리지 않는다(ripgrep `--smart-case`).
fn matcher(query: &str) -> Option<RegexMatcher> {
    RegexMatcherBuilder::new()
        .fixed_strings(true)
        .case_smart(true)
        .build(query)
        .ok()
}

/// 시트들에서 검색어를 찾아 든 시트만 주어진 순서대로 돌려준다. 빈 검색어는 아무것도 찾지 않는다.
pub fn search_sheets(paths: &[PathBuf], query: &str) -> Vec<SearchHit> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }
    let Some(matcher) = matcher(query) else {
        return Vec::new();
    };
    // 줄 번호는 쓰지 않지만 `Lossy` 싱크가 켜 두기를 요구한다.
    let mut searcher = SearcherBuilder::new()
        .binary_detection(BinaryDetection::quit(0))
        .line_number(true)
        .build();
    paths
        .iter()
        .filter_map(|path| {
            let mut hit: Option<SearchHit> = None;
            let sink = Lossy(|_, line| {
                match &mut hit {
                    Some(hit) => hit.lines += 1,
                    None => hit = Some(first_hit(path, &matcher, line)),
                }
                Ok(true)
            });
            // 읽지 못한 시트는 결과에서 뺀다.
            searcher.search_path(&matcher, path, sink).ok()?;
            hit
        })
        .collect()
}

/// 첫 일치 줄로 결과를 만든다. 긴 줄은 첫 일치 앞뒤만 남긴다.
fn first_hit(path: &Path, matcher: &RegexMatcher, line: &str) -> SearchHit {
    let line = line.trim_end_matches(['\r', '\n']);
    let mut found = Vec::new();
    let _ = matcher.find_iter(line.as_bytes(), |m| {
        found.push(m.start()..m.end());
        true
    });
    let first = found.first().map_or(0, |range| range.start);
    // 앞뒤를 글자 경계에서 자른다.
    let start = line[..first]
        .char_indices()
        .rev()
        .nth(SNIPPET_BEFORE.saturating_sub(1))
        .map_or(0, |(index, _)| index);
    let end = line[start..]
        .char_indices()
        .nth(SNIPPET_CHARS)
        .map_or(line.len(), |(index, _)| start + index);
    // 앞쪽 공백(들여쓰기)은 버린다.
    let body = &line[start..end];
    let trimmed = body.trim_start();
    let start = start + (body.len() - trimmed.len());
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < line.len() { "…" } else { "" };
    let shift = prefix.len();
    let ranges = found
        .into_iter()
        .filter(|range| range.start >= start && range.end <= end)
        .map(|range| range.start - start + shift..range.end - start + shift)
        .collect();
    SearchHit {
        path: path.to_path_buf(),
        lines: 1,
        snippet: format!("{prefix}{}{suffix}", &line[start..end]),
        ranges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn finds_sheets_in_order_with_line_counts() {
        let dir = crate::library::test_util::temp_dir("search-order");
        let a = write(&dir, "a.md", "# 제목\n\n고양이가 잔다.\n고양이가 먹는다.\n");
        let b = write(&dir, "b.md", "개가 짖는다.\n");
        let c = write(&dir, "c.md", "검은 고양이\n");
        let hits = search_sheets(&[c.clone(), a.clone(), b], "고양이");
        assert_eq!(
            hits.iter().map(|hit| &hit.path).collect::<Vec<_>>(),
            [&c, &a]
        );
        assert_eq!(hits[1].lines, 2);
        assert_eq!(hits[1].snippet, "고양이가 잔다.");
        assert_eq!(hits[1].ranges, [0..9]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn smart_case_and_literal_query() {
        let dir = crate::library::test_util::temp_dir("search-case");
        let a = write(&dir, "a.md", "Hello World (draft)\n");
        assert_eq!(search_sheets(&[a.clone()], "hello").len(), 1);
        assert!(search_sheets(&[a.clone()], "HELLO").is_empty());
        // 정규식 기호도 글자 그대로 찾는다.
        assert_eq!(search_sheets(&[a.clone()], "(draft)").len(), 1);
        assert!(search_sheets(&[a], "  ").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn long_line_snippet_keeps_match_in_view() {
        let dir = crate::library::test_util::temp_dir("search-snippet");
        let line = format!("{}바늘{}", "가".repeat(100), "나".repeat(300));
        let a = write(&dir, "a.md", &line);
        let hit = &search_sheets(&[a], "바늘")[0];
        assert!(hit.snippet.starts_with('…') && hit.snippet.ends_with('…'));
        assert_eq!(hit.snippet.chars().count(), SNIPPET_CHARS + 2);
        let range = hit.ranges[0].clone();
        assert_eq!(&hit.snippet[range], "바늘");
        let _ = std::fs::remove_dir_all(dir);
    }
}
