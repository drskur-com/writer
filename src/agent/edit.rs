//! `edit_file`의 글 고치기: 원문 조각을 찾아 새 글로 바꾼다.
//!
//! - 고칠 곳 여럿을 한 번에 받는다. 모두 원래 글에서 찾고, 하나라도 못 찾거나 겹치면 아무것도
//!   바꾸지 않는다.
//! - 원문은 글 안에 한 번만 나와야 한다(`all`이면 나오는 곳 모두).
//! - 그대로 찾지 못하면 표기를 맞춰 다시 찾는다: 둥근 따옴표 ↔ 곧은 따옴표, `…` ↔ `...`,
//!   줄바꿈 없는 공백 ↔ 공백, 한글 자모 분해(NFD) ↔ 조합(NFC). 바꾸는 범위는 원래 글의 것이다.

use std::ops::Range;

use unicode_normalization::UnicodeNormalization as _;

/// 고칠 곳 하나.
#[derive(Clone, Debug)]
pub struct Edit {
    pub old: String,
    pub new: String,
    /// 나오는 곳을 모두 바꾼다.
    pub all: bool,
}

/// 고친 결과.
#[derive(Debug, PartialEq, Eq)]
pub struct Edited {
    pub text: String,
    /// 고칠 곳마다 (원래 글에서의 줄 번호(1부터), 바꾼 곳 수).
    pub places: Vec<(usize, usize)>,
}

/// `edits`를 `text`에 적용한다. 실패하면 모델이 고쳐 다시 부를 수 있게 까닭을 돌려준다.
pub fn apply(text: &str, edits: &[Edit]) -> Result<Edited, String> {
    if edits.is_empty() {
        return Err("No edits given.".into());
    }
    let mut ranges: Vec<(Range<usize>, usize)> = Vec::new();
    let mut places = Vec::with_capacity(edits.len());
    for (ix, edit) in edits.iter().enumerate() {
        let label = format!("Edit {}", ix + 1);
        if edit.old.is_empty() {
            return Err(format!("{label}: old is empty."));
        }
        if edit.old == edit.new {
            return Err(format!("{label}: old and new are the same."));
        }
        let found = find(text, &edit.old);
        match found.len() {
            0 => {
                return Err(format!(
                    "{label}: old was not found.{}",
                    nearest_hint(text, &edit.old)
                ));
            }
            1 => {}
            count if !edit.all => {
                let lines: Vec<String> = found
                    .iter()
                    .take(5)
                    .map(|range| line_of(text, range.start).to_string())
                    .collect();
                return Err(format!(
                    "{label}: old occurs {count} times (lines {}). Add surrounding text so it matches \
                     one place, or set all to true to replace every occurrence.",
                    lines.join(", ")
                ));
            }
            _ => {}
        }
        places.push((line_of(text, found[0].start), found.len()));
        ranges.extend(found.into_iter().map(|range| (range, ix)));
    }
    ranges.sort_by_key(|(range, _)| range.start);
    if let Some(pair) = ranges
        .windows(2)
        .find(|pair| pair[0].0.end > pair[1].0.start)
    {
        return Err(format!(
            "Edits {} and {} overlap. Merge them into one.",
            pair[0].1 + 1,
            pair[1].1 + 1
        ));
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (range, ix) in &ranges {
        out.push_str(&text[at..range.start]);
        out.push_str(&edits[*ix].new);
        at = range.end;
    }
    out.push_str(&text[at..]);
    Ok(Edited { text: out, places })
}

/// `needle`이 나오는 곳(바이트 범위). 그대로 찾고, 없으면 표기를 맞춰 찾는다.
fn find(text: &str, needle: &str) -> Vec<Range<usize>> {
    let exact: Vec<_> = text
        .match_indices(needle)
        .map(|(start, found)| start..start + found.len())
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    let (hay, offsets) = normalized_with_offsets(text);
    let needle: Vec<char> = normalized(needle);
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    // 원래 글의 글자 경계에서 시작하고 끝나는 것만 쓴다(`…`의 가운데에서 끊기지 않게).
    let starts_char = |i: usize| i == 0 || offsets[i - 1] != offsets[i];
    let ends_char = |j: usize| j == hay.len() || offsets[j] != offsets[j - 1];
    let mut found = Vec::new();
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        let j = i + needle.len();
        if hay[i..j] == needle[..] && starts_char(i) && ends_char(j) {
            let end = offsets.get(j).copied().unwrap_or(text.len());
            found.push(offsets[i]..end);
            i = j;
        } else {
            i += 1;
        }
    }
    found
}

/// 비교용으로 표기를 맞춘 글자들.
fn normalized(text: &str) -> Vec<char> {
    normalized_with_offsets(text).0
}

/// 비교용 글자들과, 글자마다 그것이 나온 원래 글의 바이트 위치.
fn normalized_with_offsets(text: &str) -> (Vec<char>, Vec<usize>) {
    let mut chars = Vec::with_capacity(text.len());
    let mut offsets = Vec::with_capacity(text.len());
    let mut push = |c: char, at: usize| {
        // 자모를 분해해 맞춘다(조합은 글자 수가 바뀌어 위치를 잇기 어렵다).
        for d in std::iter::once(c).nfd() {
            chars.push(d);
            offsets.push(at);
        }
    };
    for (at, c) in text.char_indices() {
        match c {
            '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{2032}' => push('\'', at),
            '\u{201c}' | '\u{201d}' | '\u{201e}' | '\u{2033}' => push('"', at),
            '\u{2026}' => "...".chars().for_each(|dot| push(dot, at)),
            '\u{00a0}' | '\u{202f}' | '\u{2007}' => push(' ', at),
            _ => push(c, at),
        }
    }
    (chars, offsets)
}

/// `at`(바이트 위치)이 든 줄 번호(1부터).
fn line_of(text: &str, at: usize) -> usize {
    text[..at].matches('\n').count() + 1
}

/// 못 찾았을 때 붙일 힌트: `needle`의 첫 줄과 가장 비슷한 줄.
fn nearest_hint(text: &str, needle: &str) -> String {
    const MAX_CHARS: usize = 200;
    let first = needle
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(needle);
    let target = bigrams(first);
    if target.is_empty() {
        return String::new();
    }
    let best = text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(ix, line)| {
            let grams = bigrams(line);
            let shared = target.iter().filter(|gram| grams.contains(gram)).count();
            (shared, ix, line)
        })
        .max_by_key(|(shared, ix, _)| (*shared, std::cmp::Reverse(*ix)));
    match best {
        // 절반도 겹치지 않으면 엉뚱한 줄이다.
        Some((shared, ix, line)) if shared * 2 >= target.len() => {
            let shown: String = line.chars().take(MAX_CHARS).collect();
            let more = if line.chars().count() > MAX_CHARS {
                "…"
            } else {
                ""
            };
            format!(" Closest line (line {}): {shown}{more}", ix + 1)
        }
        _ => " The file may have changed; read its current content with read_file.".into(),
    }
}

fn bigrams(text: &str) -> Vec<(char, char)> {
    let chars = normalized(text);
    let mut grams: Vec<_> = chars.windows(2).map(|pair| (pair[0], pair[1])).collect();
    grams.sort_unstable();
    grams.dedup();
    grams
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(old: &str, new: &str) -> Edit {
        Edit {
            old: old.into(),
            new: new.into(),
            all: false,
        }
    }

    #[test]
    fn applies_several_edits_at_once() {
        let text = "첫 문장이다.\n\n둘째 문장이다.\n\n셋째 문장이다.\n";
        let edited = apply(text, &[edit("셋째", "세째"), edit("첫 문장", "처음 문장")]).unwrap();
        assert_eq!(
            edited.text,
            "처음 문장이다.\n\n둘째 문장이다.\n\n세째 문장이다.\n"
        );
        assert_eq!(edited.places, [(5, 1), (1, 1)]);
    }

    #[test]
    fn refuses_ambiguous_missing_or_overlapping() {
        let text = "가나다 가나다";
        let err = apply(text, &[edit("가나다", "라")]).unwrap_err();
        assert!(err.contains("2 times"), "{err}");
        let all = Edit {
            all: true,
            ..edit("가나다", "라")
        };
        assert_eq!(apply(text, &[all]).unwrap().text, "라 라");
        assert!(
            apply(text, &[edit("없음", "x")])
                .unwrap_err()
                .contains("not found")
        );
        let err = apply(text, &[edit("가나다 가", "x"), edit("다 가나다", "y")]).unwrap_err();
        assert!(err.contains("overlap"), "{err}");
        assert!(apply(text, &[edit("", "x")]).is_err());
        assert!(apply(text, &[]).is_err());
    }

    #[test]
    fn matches_typographic_and_decomposed_text() {
        // 파일은 둥근 따옴표·말줄임표, 모델은 곧은 따옴표·마침표 셋.
        let text = "그는 “안녕…”이라고 했다.";
        let edited = apply(text, &[edit("\"안녕...\"이라고", "“잘 가”라고")]).unwrap();
        assert_eq!(edited.text, "그는 “잘 가”라고 했다.");
        // 파일은 자모가 분해된 한글(NFD), 모델은 조합된 한글(NFC).
        let nfd: String = "한글 문장".nfd().collect();
        let edited = apply(&nfd, &[edit("글 문", "국어 문")]).unwrap();
        assert_eq!(
            edited.text,
            format!(
                "{}국어 문{}",
                "한".nfd().collect::<String>(),
                "장".nfd().collect::<String>()
            )
        );
        // 말줄임표 가운데에서 끊기는 것은 맞지 않는다.
        assert!(apply("기다려…", &[edit("기다려.", "x")]).is_err());
    }

    #[test]
    fn hints_nearest_line() {
        let text = "첫 줄\n바다는 넓고 푸르다\n끝";
        let err = apply(text, &[edit("바다는 넓고 파랗다", "x")]).unwrap_err();
        assert!(err.contains("(line 2): 바다는 넓고 푸르다"), "{err}");
    }
}
