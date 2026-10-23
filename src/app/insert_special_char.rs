//! 특수 문자 및 이모지 삽입 대화상자: 조판 공백, 대시, 낫표/겹낫표, 기호, 이모지 등을 선택해 삽입한다.

use gpui_base::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{EditorState, Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, FocusableExt as _, Icon, Sizable as _, WindowExt as _,
};
use gpui_kit::gpui::prelude::{FluentBuilder as _, InteractiveElement as _};
use gpui_kit::gpui::*;
use std::cell::Cell;
use std::rc::Rc;

use super::WriterApp;
use super::style::Palette;
use crate::{InsertSpecialChar, icons};

/// 대화상자 기본 너비.
const DIALOG_WIDTH: f32 = 600.0;
/// 격자 칸 한 개의 높이(넓은 이름표는 이 높이에 글자 폭만큼만 넓어진다).
const TILE_HEIGHT: f32 = 38.0;
/// 격자 영역 높이. 범주를 바꿔도 대화상자 높이가 흔들리지 않도록 고정한다.
const GRID_HEIGHT: f32 = 248.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CharCategory {
    All,
    Typesetting,
    Quotes,
    Symbols,
    Emoji,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SpecialCharItem {
    pub display: &'static str,
    pub insert: &'static str,
    /// 이름·설명의 번역 키(`locales/special_char.yml`).
    pub name: &'static str,
    pub desc: &'static str,
    pub category: CharCategory,
    pub keywords: &'static str,
    pub is_pair: bool,
    pub pair_open_len: usize,
}

const fn item(
    display: &'static str,
    insert: &'static str,
    name: &'static str,
    desc: &'static str,
    category: CharCategory,
    keywords: &'static str,
) -> SpecialCharItem {
    SpecialCharItem {
        display,
        insert,
        name,
        desc,
        category,
        keywords,
        is_pair: false,
        pair_open_len: 0,
    }
}

const fn pair(
    display: &'static str,
    insert: &'static str,
    name: &'static str,
    desc: &'static str,
    category: CharCategory,
    keywords: &'static str,
    pair_open_len: usize,
) -> SpecialCharItem {
    SpecialCharItem {
        display,
        insert,
        name,
        desc,
        category,
        keywords,
        is_pair: true,
        pair_open_len,
    }
}

/// 칸에 그릴 내용의 종류. 글자 수·바이트 수가 아니라 보이는 글자 수로 정한다.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TileKind {
    /// `NBSP`, `ZWSP`처럼 글자 대신 이름을 쓰는 칸.
    Abbr,
    /// `「…」`처럼 괄호 싸개를 보여 주는 칸.
    Pair,
    /// 글자 하나(이모지·문장부호).
    Glyph,
}

/// 이모지의 변이 선택자·결합자를 뺀, 실제로 보이는 글자 수.
fn glyph_count(display: &str) -> usize {
    display
        .chars()
        .filter(|c| !matches!(c, '\u{FE0E}' | '\u{FE0F}' | '\u{200D}'))
        .count()
}

fn tile_kind(item: &SpecialCharItem) -> TileKind {
    let ascii_name = item.display.len() >= 2
        && item
            .display
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ');
    if ascii_name {
        TileKind::Abbr
    } else if glyph_count(item.display) <= 1 {
        TileKind::Glyph
    } else {
        TileKind::Pair
    }
}

/// 설명줄에 보여 줄 코드 포인트(`U+2009`). 싸개는 여는 글자 기준.
pub(crate) fn code_point(item: &SpecialCharItem) -> String {
    item.insert
        .chars()
        .next()
        .map(|c| format!("U+{:04X}", c as u32))
        .unwrap_or_default()
}

pub(crate) static SPECIAL_CHARS: &[SpecialCharItem] = &[
    // --- 조판 및 공백 (Typesetting & Spaces) ---
    item(
        "NBSP",
        "\u{00A0}",
        "special_char.u00a0.name",
        "special_char.u00a0.desc",
        CharCategory::Typesetting,
        "공백 nbsp space 줄바꿈 non-breaking",
    ),
    item(
        "Thin",
        "\u{2009}",
        "special_char.u2009.name",
        "special_char.u2009.desc",
        CharCategory::Typesetting,
        "공백 thin space 얇은",
    ),
    item(
        "En Sp",
        "\u{2002}",
        "special_char.u2002.name",
        "special_char.u2002.desc",
        CharCategory::Typesetting,
        "공백 en space 반각",
    ),
    item(
        "Em Sp",
        "\u{2003}",
        "special_char.u2003.name",
        "special_char.u2003.desc",
        CharCategory::Typesetting,
        "공백 em space 전각",
    ),
    item(
        "ZWSP",
        "\u{200B}",
        "special_char.u200b.name",
        "special_char.u200b.desc",
        CharCategory::Typesetting,
        "공백 zwsp zero width space 줄바꿈",
    ),
    item(
        "WJ",
        "\u{2060}",
        "special_char.u2060.name",
        "special_char.u2060.desc",
        CharCategory::Typesetting,
        "공백 결합방지 wj word joiner",
    ),
    item(
        "SHY",
        "\u{00AD}",
        "special_char.u00ad.name",
        "special_char.u00ad.desc",
        CharCategory::Typesetting,
        "하이픈 shy soft hyphen 붙임표",
    ),
    item(
        "‑",
        "\u{2011}",
        "special_char.u2011.name",
        "special_char.u2011.desc",
        CharCategory::Typesetting,
        "하이픈 non-breaking hyphen 붙임표",
    ),
    item(
        "—",
        "—",
        "special_char.u2014.name",
        "special_char.u2014.desc",
        CharCategory::Typesetting,
        "대시 em dash 줄표",
    ),
    item(
        "–",
        "–",
        "special_char.u2013.name",
        "special_char.u2013.desc",
        CharCategory::Typesetting,
        "대시 en dash 줄표 기간 범위",
    ),
    item(
        "…",
        "…",
        "special_char.u2026.name",
        "special_char.u2026.desc",
        CharCategory::Typesetting,
        "말줄임표 ellipsis 점 리더",
    ),
    item(
        "‥",
        "‥",
        "special_char.u2025.name",
        "special_char.u2025.desc",
        CharCategory::Typesetting,
        "말줄임표 이점 점",
    ),
    item(
        "·",
        "·",
        "special_char.u00b7.name",
        "special_char.u00b7.desc",
        CharCategory::Typesetting,
        "가운뎃점 middle dot 점",
    ),
    item(
        "•",
        "•",
        "special_char.u2022.name",
        "special_char.u2022.desc",
        CharCategory::Typesetting,
        "불릿 bullet 목록 점",
    ),
    item(
        "※",
        "※",
        "special_char.u203b.name",
        "special_char.u203b.desc",
        CharCategory::Typesetting,
        "참고표 당구장 주석",
    ),
    item(
        "¶",
        "¶",
        "special_char.u00b6.name",
        "special_char.u00b6.desc",
        CharCategory::Typesetting,
        "단락 문단 pilcrow paragraph",
    ),
    item(
        "§",
        "§",
        "special_char.u00a7.name",
        "special_char.u00a7.desc",
        CharCategory::Typesetting,
        "절 섹션 section 조항",
    ),
    item(
        "†",
        "†",
        "special_char.u2020.name",
        "special_char.u2020.desc",
        CharCategory::Typesetting,
        "칼표 dagger 각주",
    ),
    item(
        "‡",
        "‡",
        "special_char.u2021.name",
        "special_char.u2021.desc",
        CharCategory::Typesetting,
        "칼표 double dagger 각주",
    ),
    // --- 따옴표 및 괄호 (Quotes & Brackets) ---
    pair(
        "「…」",
        "「」",
        "special_char.u300c_300d.name",
        "special_char.u300c_300d.desc",
        CharCategory::Quotes,
        "낫표 괄호 인용",
        3,
    ),
    pair(
        "『…』",
        "『』",
        "special_char.u300e_300f.name",
        "special_char.u300e_300f.desc",
        CharCategory::Quotes,
        "겹낫표 책제목 괄호 인용",
        3,
    ),
    pair(
        "〈…〉",
        "〈〉",
        "special_char.u3008_3009.name",
        "special_char.u3008_3009.desc",
        CharCategory::Quotes,
        "홑화살괄호 꺾쇠 괄호",
        3,
    ),
    pair(
        "《…》",
        "《》",
        "special_char.u300a_300b.name",
        "special_char.u300a_300b.desc",
        CharCategory::Quotes,
        "겹화살괄호 책제목 꺾쇠",
        3,
    ),
    pair(
        "“…”",
        "“”",
        "special_char.u201c_201d.name",
        "special_char.u201c_201d.desc",
        CharCategory::Quotes,
        "큰따옴표 quote 인용",
        3,
    ),
    pair(
        "‘…’",
        "‘’",
        "special_char.u2018_2019.name",
        "special_char.u2018_2019.desc",
        CharCategory::Quotes,
        "작은따옴표 quote 인용",
        3,
    ),
    pair(
        "«…»",
        "«»",
        "special_char.u00ab_00bb.name",
        "special_char.u00ab_00bb.desc",
        CharCategory::Quotes,
        "기예메 guillemet 인용",
        2,
    ),
    pair(
        "【…】",
        "【】",
        "special_char.u3010_3011.name",
        "special_char.u3010_3011.desc",
        CharCategory::Quotes,
        "볼드 꺾쇠 괄호",
        3,
    ),
    pair(
        "〔…〕",
        "〔〕",
        "special_char.u3014_3015.name",
        "special_char.u3014_3015.desc",
        CharCategory::Quotes,
        "거북등 괄호 귀호",
        3,
    ),
    item(
        "「",
        "「",
        "special_char.u300c.name",
        "special_char.u300c.desc",
        CharCategory::Quotes,
        "낫표 괄호 인용",
    ),
    item(
        "」",
        "」",
        "special_char.u300d.name",
        "special_char.u300d.desc",
        CharCategory::Quotes,
        "낫표 괄호 인용",
    ),
    item(
        "『",
        "『",
        "special_char.u300e.name",
        "special_char.u300e.desc",
        CharCategory::Quotes,
        "겹낫표 괄호 책제목",
    ),
    item(
        "』",
        "』",
        "special_char.u300f.name",
        "special_char.u300f.desc",
        CharCategory::Quotes,
        "겹낫표 괄호 책제목",
    ),
    item(
        "〈",
        "〈",
        "special_char.u3008.name",
        "special_char.u3008.desc",
        CharCategory::Quotes,
        "홑화살괄호 꺾쇠",
    ),
    item(
        "〉",
        "〉",
        "special_char.u3009.name",
        "special_char.u3009.desc",
        CharCategory::Quotes,
        "홑화살괄호 꺾쇠",
    ),
    item(
        "《",
        "《",
        "special_char.u300a.name",
        "special_char.u300a.desc",
        CharCategory::Quotes,
        "겹화살괄호 꺾쇠 책제목",
    ),
    item(
        "》",
        "》",
        "special_char.u300b.name",
        "special_char.u300b.desc",
        CharCategory::Quotes,
        "겹화살괄호 꺾쇠 책제목",
    ),
    item(
        "“",
        "“",
        "special_char.u201c.name",
        "special_char.u201c.desc",
        CharCategory::Quotes,
        "큰따옴표 quote 인용",
    ),
    item(
        "”",
        "”",
        "special_char.u201d.name",
        "special_char.u201d.desc",
        CharCategory::Quotes,
        "큰따옴표 quote 인용",
    ),
    item(
        "‘",
        "‘",
        "special_char.u2018.name",
        "special_char.u2018.desc",
        CharCategory::Quotes,
        "작은따옴표 quote 인용",
    ),
    item(
        "’",
        "’",
        "special_char.u2019.name",
        "special_char.u2019.desc",
        CharCategory::Quotes,
        "작은따옴표 quote 인용",
    ),
    // --- 기호 및 단위 (Symbols & Units) ---
    item(
        "©",
        "©",
        "special_char.u00a9.name",
        "special_char.u00a9.desc",
        CharCategory::Symbols,
        "저작권 copyright copy c",
    ),
    item(
        "®",
        "®",
        "special_char.u00ae.name",
        "special_char.u00ae.desc",
        CharCategory::Symbols,
        "등록상표 registered r tm",
    ),
    item(
        "™",
        "™",
        "special_char.u2122.name",
        "special_char.u2122.desc",
        CharCategory::Symbols,
        "상표 trademark tm",
    ),
    item(
        "°",
        "°",
        "special_char.u00b0.name",
        "special_char.u00b0.desc",
        CharCategory::Symbols,
        "도 degree 각도 온도",
    ),
    item(
        "′",
        "′",
        "special_char.u2032.name",
        "special_char.u2032.desc",
        CharCategory::Symbols,
        "프라임 분 prime foot",
    ),
    item(
        "″",
        "″",
        "special_char.u2033.name",
        "special_char.u2033.desc",
        CharCategory::Symbols,
        "더블프라임 초 inch double prime",
    ),
    item(
        "℃",
        "℃",
        "special_char.u2103.name",
        "special_char.u2103.desc",
        CharCategory::Symbols,
        "온도 섭씨 celcius",
    ),
    item(
        "℉",
        "℉",
        "special_char.u2109.name",
        "special_char.u2109.desc",
        CharCategory::Symbols,
        "온도 화씨 fahrenheit",
    ),
    item(
        "‰",
        "‰",
        "special_char.u2030.name",
        "special_char.u2030.desc",
        CharCategory::Symbols,
        "퍼밀 천분율 permil",
    ),
    item(
        "₩",
        "₩",
        "special_char.u20a9.name",
        "special_char.u20a9.desc",
        CharCategory::Symbols,
        "원 통화 money won krw",
    ),
    item(
        "$",
        "$",
        "special_char.u0024.name",
        "special_char.u0024.desc",
        CharCategory::Symbols,
        "달러 통화 money dollar usd",
    ),
    item(
        "€",
        "€",
        "special_char.u20ac.name",
        "special_char.u20ac.desc",
        CharCategory::Symbols,
        "유로 통화 money euro eur",
    ),
    item(
        "£",
        "£",
        "special_char.u00a3.name",
        "special_char.u00a3.desc",
        CharCategory::Symbols,
        "파운드 통화 money pound gbp",
    ),
    item(
        "¥",
        "¥",
        "special_char.u00a5.name",
        "special_char.u00a5.desc",
        CharCategory::Symbols,
        "엔 위안 통화 yen yuan",
    ),
    item(
        "±",
        "±",
        "special_char.u00b1.name",
        "special_char.u00b1.desc",
        CharCategory::Symbols,
        "플러스마이너스 plusminus 오차",
    ),
    item(
        "×",
        "×",
        "special_char.u00d7.name",
        "special_char.u00d7.desc",
        CharCategory::Symbols,
        "곱셈 치수 크기 multiply",
    ),
    item(
        "÷",
        "÷",
        "special_char.u00f7.name",
        "special_char.u00f7.desc",
        CharCategory::Symbols,
        "나눗셈 divide",
    ),
    item(
        "≠",
        "≠",
        "special_char.u2260.name",
        "special_char.u2260.desc",
        CharCategory::Symbols,
        "같지않음 다름 not equal",
    ),
    item(
        "≤",
        "≤",
        "special_char.u2264.name",
        "special_char.u2264.desc",
        CharCategory::Symbols,
        "작거나같음 이하 less equal",
    ),
    item(
        "≥",
        "≥",
        "special_char.u2265.name",
        "special_char.u2265.desc",
        CharCategory::Symbols,
        "크거나같음 이상 greater equal",
    ),
    item(
        "≈",
        "≈",
        "special_char.u2248.name",
        "special_char.u2248.desc",
        CharCategory::Symbols,
        "근사 약 approx",
    ),
    item(
        "∞",
        "∞",
        "special_char.u221e.name",
        "special_char.u221e.desc",
        CharCategory::Symbols,
        "무한대 infinity",
    ),
    item(
        "√",
        "√",
        "special_char.u221a.name",
        "special_char.u221a.desc",
        CharCategory::Symbols,
        "루트 root",
    ),
    item(
        "←",
        "←",
        "special_char.u2190.name",
        "special_char.u2190.desc",
        CharCategory::Symbols,
        "화살표 arrow left",
    ),
    item(
        "→",
        "→",
        "special_char.u2192.name",
        "special_char.u2192.desc",
        CharCategory::Symbols,
        "화살표 arrow right",
    ),
    item(
        "↑",
        "↑",
        "special_char.u2191.name",
        "special_char.u2191.desc",
        CharCategory::Symbols,
        "화살표 arrow up",
    ),
    item(
        "↓",
        "↓",
        "special_char.u2193.name",
        "special_char.u2193.desc",
        CharCategory::Symbols,
        "화살표 arrow down",
    ),
    item(
        "↔",
        "↔",
        "special_char.u2194.name",
        "special_char.u2194.desc",
        CharCategory::Symbols,
        "화살표 arrow both",
    ),
    item(
        "⇒",
        "⇒",
        "special_char.u21d2.name",
        "special_char.u21d2.desc",
        CharCategory::Symbols,
        "화살표 implies arrow",
    ),
    item(
        "✓",
        "✓",
        "special_char.u2713.name",
        "special_char.u2713.desc",
        CharCategory::Symbols,
        "체크 check mark",
    ),
    item(
        "★",
        "★",
        "special_char.u2605.name",
        "special_char.u2605.desc",
        CharCategory::Symbols,
        "별 star",
    ),
    item(
        "☆",
        "☆",
        "special_char.u2606.name",
        "special_char.u2606.desc",
        CharCategory::Symbols,
        "별 star",
    ),
    item(
        "◆",
        "◆",
        "special_char.u25c6.name",
        "special_char.u25c6.desc",
        CharCategory::Symbols,
        "다이아몬드 diamond",
    ),
    item(
        "◇",
        "◇",
        "special_char.u25c7.name",
        "special_char.u25c7.desc",
        CharCategory::Symbols,
        "다이아몬드 diamond",
    ),
    item(
        "■",
        "■",
        "special_char.u25a0.name",
        "special_char.u25a0.desc",
        CharCategory::Symbols,
        "네모 square",
    ),
    item(
        "□",
        "□",
        "special_char.u25a1.name",
        "special_char.u25a1.desc",
        CharCategory::Symbols,
        "네모 square",
    ),
    // --- 이모지 (Emoji) ---
    item(
        "📝",
        "📝",
        "special_char.u1f4dd.name",
        "special_char.u1f4dd.desc",
        CharCategory::Emoji,
        "메모 memo note 글쓰기",
    ),
    item(
        "📖",
        "📖",
        "special_char.u1f4d6.name",
        "special_char.u1f4d6.desc",
        CharCategory::Emoji,
        "책 book read 독서",
    ),
    item(
        "📚",
        "📚",
        "special_char.u1f4da.name",
        "special_char.u1f4da.desc",
        CharCategory::Emoji,
        "책 서적 books library",
    ),
    item(
        "📄",
        "📄",
        "special_char.u1f4c4.name",
        "special_char.u1f4c4.desc",
        CharCategory::Emoji,
        "문서 doc page paper",
    ),
    item(
        "🖋️",
        "🖋️",
        "special_char.u1f58b.name",
        "special_char.u1f58b.desc",
        CharCategory::Emoji,
        "펜 만년필 pen fountain",
    ),
    item(
        "✏️",
        "✏️",
        "special_char.u270f.name",
        "special_char.u270f.desc",
        CharCategory::Emoji,
        "연필 pencil write",
    ),
    item(
        "📌",
        "📌",
        "special_char.u1f4cc.name",
        "special_char.u1f4cc.desc",
        CharCategory::Emoji,
        "압정 pin 고정",
    ),
    item(
        "📍",
        "📍",
        "special_char.u1f4cd.name",
        "special_char.u1f4cd.desc",
        CharCategory::Emoji,
        "핀 pin 위치",
    ),
    item(
        "📎",
        "📎",
        "special_char.u1f4ce.name",
        "special_char.u1f4ce.desc",
        CharCategory::Emoji,
        "클립 clip 첨부",
    ),
    item(
        "🔖",
        "🔖",
        "special_char.u1f516.name",
        "special_char.u1f516.desc",
        CharCategory::Emoji,
        "책갈피 bookmark 북마크",
    ),
    item(
        "💡",
        "💡",
        "special_char.u1f4a1.name",
        "special_char.u1f4a1.desc",
        CharCategory::Emoji,
        "전구 아이디어 idea lightbulb",
    ),
    item(
        "💭",
        "💭",
        "special_char.u1f4ad.name",
        "special_char.u1f4ad.desc",
        CharCategory::Emoji,
        "생각 thought bubble",
    ),
    item(
        "💬",
        "💬",
        "special_char.u1f4ac.name",
        "special_char.u1f4ac.desc",
        CharCategory::Emoji,
        "말풍선 comment speech chat",
    ),
    item(
        "✍️",
        "✍️",
        "special_char.u270d.name",
        "special_char.u270d.desc",
        CharCategory::Emoji,
        "글쓰기 write hand 서명",
    ),
    item(
        "🗓️",
        "🗓️",
        "special_char.u1f5d3.name",
        "special_char.u1f5d3.desc",
        CharCategory::Emoji,
        "달력 calendar 날짜",
    ),
    item(
        "⏱️",
        "⏱️",
        "special_char.u23f1.name",
        "special_char.u23f1.desc",
        CharCategory::Emoji,
        "시간 timer stopwatch 시계",
    ),
    item(
        "⏳",
        "⏳",
        "special_char.u23f3.name",
        "special_char.u23f3.desc",
        CharCategory::Emoji,
        "모래시계 hourglass 시간",
    ),
    item(
        "😀",
        "😀",
        "special_char.u1f600.name",
        "special_char.u1f600.desc",
        CharCategory::Emoji,
        "웃음 smile grin",
    ),
    item(
        "😊",
        "😊",
        "special_char.u1f60a.name",
        "special_char.u1f60a.desc",
        CharCategory::Emoji,
        "미소 smile happy",
    ),
    item(
        "🥰",
        "🥰",
        "special_char.u1f970.name",
        "special_char.u1f970.desc",
        CharCategory::Emoji,
        "하트 love heart",
    ),
    item(
        "😍",
        "😍",
        "special_char.u1f60d.name",
        "special_char.u1f60d.desc",
        CharCategory::Emoji,
        "하트 반함 love heart eyes",
    ),
    item(
        "🤔",
        "🤔",
        "special_char.u1f914.name",
        "special_char.u1f914.desc",
        CharCategory::Emoji,
        "생각 고민 thinking think",
    ),
    item(
        "🧐",
        "🧐",
        "special_char.u1f9d0.name",
        "special_char.u1f9d0.desc",
        CharCategory::Emoji,
        "관찰 monocle inspect",
    ),
    item(
        "🤫",
        "🤫",
        "special_char.u1f92b.name",
        "special_char.u1f92b.desc",
        CharCategory::Emoji,
        "쉿 quiet shh",
    ),
    item(
        "😅",
        "😅",
        "special_char.u1f605.name",
        "special_char.u1f605.desc",
        CharCategory::Emoji,
        "땀 sweat sweat smile",
    ),
    item(
        "😢",
        "😢",
        "special_char.u1f622.name",
        "special_char.u1f622.desc",
        CharCategory::Emoji,
        "눈물 슬픔 cry sad",
    ),
    item(
        "😭",
        "😭",
        "special_char.u1f62d.name",
        "special_char.u1f62d.desc",
        CharCategory::Emoji,
        "눈물 슬픔 sob crying",
    ),
    item(
        "🥳",
        "🥳",
        "special_char.u1f973.name",
        "special_char.u1f973.desc",
        CharCategory::Emoji,
        "파티 축하 party celebrate",
    ),
    item(
        "😎",
        "😎",
        "special_char.u1f60e.name",
        "special_char.u1f60e.desc",
        CharCategory::Emoji,
        "선글라스 cool sunglasses",
    ),
    item(
        "🤩",
        "🤩",
        "special_char.u1f929.name",
        "special_char.u1f929.desc",
        CharCategory::Emoji,
        "스타 star eyes 반짝",
    ),
    item(
        "👍",
        "👍",
        "special_char.u1f44d.name",
        "special_char.u1f44d.desc",
        CharCategory::Emoji,
        "좋아요 thumbs up good",
    ),
    item(
        "👎",
        "👎",
        "special_char.u1f44e.name",
        "special_char.u1f44e.desc",
        CharCategory::Emoji,
        "싫어요 thumbs down bad",
    ),
    item(
        "👏",
        "👏",
        "special_char.u1f44f.name",
        "special_char.u1f44f.desc",
        CharCategory::Emoji,
        "박수 clap applause",
    ),
    item(
        "🙌",
        "🙌",
        "special_char.u1f64c.name",
        "special_char.u1f64c.desc",
        CharCategory::Emoji,
        "만세 celebrate hooray",
    ),
    item(
        "🤝",
        "🤝",
        "special_char.u1f91d.name",
        "special_char.u1f91d.desc",
        CharCategory::Emoji,
        "악수 handshake deal",
    ),
    item(
        "✌️",
        "✌️",
        "special_char.u270c.name",
        "special_char.u270c.desc",
        CharCategory::Emoji,
        "브이 victory peace",
    ),
    item(
        "🤞",
        "🤞",
        "special_char.u1f91e.name",
        "special_char.u1f91e.desc",
        CharCategory::Emoji,
        "행운 luck fingers crossed",
    ),
    item(
        "✅",
        "✅",
        "special_char.u2705.name",
        "special_char.u2705.desc",
        CharCategory::Emoji,
        "체크 완료 check done",
    ),
    item(
        "❌",
        "❌",
        "special_char.u274c.name",
        "special_char.u274c.desc",
        CharCategory::Emoji,
        "엑스 취소 cross x no",
    ),
    item(
        "⚠️",
        "⚠️",
        "special_char.u26a0.name",
        "special_char.u26a0.desc",
        CharCategory::Emoji,
        "경고 warning caution",
    ),
    item(
        "ℹ️",
        "ℹ️",
        "special_char.u2139.name",
        "special_char.u2139.desc",
        CharCategory::Emoji,
        "정보 info information",
    ),
    item(
        "❓",
        "❓",
        "special_char.u2753.name",
        "special_char.u2753.desc",
        CharCategory::Emoji,
        "물음표 question mark",
    ),
    item(
        "❗",
        "❗",
        "special_char.u2757.name",
        "special_char.u2757.desc",
        CharCategory::Emoji,
        "느낌표 exclamation mark",
    ),
    item(
        "🔴",
        "🔴",
        "special_char.u1f534.name",
        "special_char.u1f534.desc",
        CharCategory::Emoji,
        "빨강 red circle",
    ),
    item(
        "🟢",
        "🟢",
        "special_char.u1f7e2.name",
        "special_char.u1f7e2.desc",
        CharCategory::Emoji,
        "초록 green circle",
    ),
    item(
        "🔵",
        "🔵",
        "special_char.u1f535.name",
        "special_char.u1f535.desc",
        CharCategory::Emoji,
        "파랑 blue circle",
    ),
    item(
        "⚪",
        "⚪",
        "special_char.u26aa.name",
        "special_char.u26aa.desc",
        CharCategory::Emoji,
        "흰색 white circle",
    ),
    item(
        "⚫",
        "⚫",
        "special_char.u26ab.name",
        "special_char.u26ab.desc",
        CharCategory::Emoji,
        "검정 black circle",
    ),
    item(
        "🚀",
        "🚀",
        "special_char.u1f680.name",
        "special_char.u1f680.desc",
        CharCategory::Emoji,
        "로켓 rocket launch",
    ),
    item(
        "✨",
        "✨",
        "special_char.u2728.name",
        "special_char.u2728.desc",
        CharCategory::Emoji,
        "반짝 sparkles star",
    ),
    item(
        "🎉",
        "🎉",
        "special_char.u1f389.name",
        "special_char.u1f389.desc",
        CharCategory::Emoji,
        "파티 폭죽 party tada",
    ),
    item(
        "☕",
        "☕",
        "special_char.u2615.name",
        "special_char.u2615.desc",
        CharCategory::Emoji,
        "커피 coffee break",
    ),
    item(
        "🍵",
        "🍵",
        "special_char.u1f375.name",
        "special_char.u1f375.desc",
        CharCategory::Emoji,
        "차 tea green tea",
    ),
    item(
        "🌿",
        "🌿",
        "special_char.u1f33f.name",
        "special_char.u1f33f.desc",
        CharCategory::Emoji,
        "새싹 허브 herb plant",
    ),
    item(
        "🔥",
        "🔥",
        "special_char.u1f525.name",
        "special_char.u1f525.desc",
        CharCategory::Emoji,
        "불 fire flame hot",
    ),
    item(
        "⭐️",
        "⭐️",
        "special_char.u2b50.name",
        "special_char.u2b50.desc",
        CharCategory::Emoji,
        "별 star rate",
    ),
    item(
        "💯",
        "💯",
        "special_char.u1f4af.name",
        "special_char.u1f4af.desc",
        CharCategory::Emoji,
        "백점 100 hundred perfect",
    ),
];

/// 특수 문자 입력 폼 뷰.
pub(super) struct SpecialCharForm {
    app: WeakEntity<WriterApp>,
    target: Entity<EditorState>,
    pub(super) search: Entity<InputState>,
    category: CharCategory,
    hovered_index: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl SpecialCharForm {
    pub(super) fn new(
        app: WeakEntity<WriterApp>,
        target: Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("special_char.search_placeholder"))
        });
        // 검색어가 바뀌면 가리키던 칸을 비운다. 아니면 설명줄이 결과에 없는 글자를 계속 보여 준다.
        let search_sub = cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.hovered_index = None;
                cx.notify();
            }
        });

        Self {
            app,
            target,
            search,
            category: CharCategory::All,
            hovered_index: None,
            _subscriptions: vec![search_sub],
        }
    }

    fn filtered_items(&self, cx: &App) -> Vec<(usize, &'static SpecialCharItem)> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        SPECIAL_CHARS
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                let matches_cat = match self.category {
                    CharCategory::All => true,
                    cat => item.category == cat,
                };
                if !matches_cat {
                    return false;
                }
                if query.is_empty() {
                    return true;
                }
                item.display.to_lowercase().contains(&query)
                    || item.insert.contains(&query)
                    // 이름·설명은 화면 언어와 상관없이 두 언어로 찾는다.
                    || ["en", "ko"].into_iter().any(|locale| {
                        t!(item.name, locale = locale).to_lowercase().contains(&query)
                            || t!(item.desc, locale = locale).to_lowercase().contains(&query)
                    })
                    || item.keywords.to_lowercase().contains(&query)
            })
            .collect()
    }

    /// 지금 가리키는 칸. 가리킨 칸이 결과에 없으면 첫 칸이다.
    fn active_index(&self, filtered: &[(usize, &'static SpecialCharItem)]) -> Option<usize> {
        self.hovered_index
            .filter(|ix| filtered.iter().any(|(global, _)| global == ix))
            .or_else(|| filtered.first().map(|(global, _)| *global))
    }

    fn active_item(&self, cx: &App) -> Option<&'static SpecialCharItem> {
        let filtered = self.filtered_items(cx);
        self.active_index(&filtered)
            .and_then(|ix| SPECIAL_CHARS.get(ix))
    }

    /// 지금 활성인 칸을 넣는다. 넣을 칸이 없으면 `false`(대화상자를 닫지 않는다).
    fn insert_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(item) = self.active_item(cx) else {
            return false;
        };
        self.insert_item(item, window, cx);
        true
    }

    fn insert_item(&mut self, item: &SpecialCharItem, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.target.clone();
        let item = *item;
        self.app
            .update(cx, |app, cx| {
                app.insert_special_char_item(&target, &item, window, cx);
            })
            .ok();
    }
}

impl Render for SpecialCharForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Palette::from_theme(cx.theme());
        let form = cx.entity().downgrade();

        let categories = [
            (CharCategory::All, "special_char.category.all"),
            (
                CharCategory::Typesetting,
                "special_char.category.typesetting",
            ),
            (CharCategory::Quotes, "special_char.category.quotes"),
            (CharCategory::Symbols, "special_char.category.symbols"),
            (CharCategory::Emoji, "special_char.category.emoji"),
        ];

        let selected_tab = categories
            .iter()
            .position(|(cat, _)| *cat == self.category)
            .unwrap_or(0);

        let tabs = TabBar::new("special-char-category")
            .segmented()
            .selected_index(selected_tab)
            .on_click(move |index: &usize, _window, cx| {
                form.update(cx, |form, cx| {
                    if let Some((cat, _)) = categories.get(*index) {
                        form.category = *cat;
                        form.hovered_index = None;
                        cx.notify();
                    }
                })
                .ok();
            })
            .children(
                categories
                    .iter()
                    .map(|(_, label)| Tab::new().flex_1().label(t!(*label))),
            );

        let filtered = self.filtered_items(cx);
        let active_index = self.active_index(&filtered);
        let active_item: Option<&'static SpecialCharItem> =
            active_index.and_then(|ix| SPECIAL_CHARS.get(ix));
        let mono = cx.theme().mono_font_family.clone();

        let tiles = if filtered.is_empty() {
            div()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .h(px(GRID_HEIGHT))
                .child(
                    Icon::new(icons::by_name("search"))
                        .small()
                        .text_color(pal.muted.opacity(0.7)),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(pal.muted)
                        .child(t!("special_char.empty")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(pal.muted.opacity(0.8))
                        .child(t!("special_char.empty_hint")),
                )
                .into_any_element()
        } else {
            let mut grid = div()
                .flex()
                .flex_wrap()
                .gap_1_5()
                .p_2()
                .h(px(GRID_HEIGHT))
                .overflow_y_scrollbar();

            for (global_idx, item) in filtered {
                let kind = tile_kind(item);
                let is_active = active_index == Some(global_idx);
                let form_for_click = cx.entity().downgrade();
                let form_for_hover = cx.entity().downgrade();
                let item_copy = *item;

                let tile = div()
                    .id(("char-tile", global_idx))
                    .flex()
                    .items_center()
                    .justify_center()
                    .h(px(TILE_HEIGHT))
                    .min_w(px(TILE_HEIGHT))
                    .px_1p5()
                    .cursor_pointer()
                    .rounded_md()
                    .border_1()
                    .border_color(pal.border)
                    .bg(pal.panel)
                    .hover(|s| s.bg(pal.hover))
                    .when(is_active, |this| this.bg(pal.hover))
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        form_for_click
                            .update(cx, |form, cx| {
                                form.insert_item(&item_copy, window, cx);
                            })
                            .ok();
                        window.close_dialog(cx);
                    })
                    .on_mouse_move(move |_, _window, cx| {
                        form_for_hover
                            .update(cx, |form, cx| {
                                if form.hovered_index != Some(global_idx) {
                                    form.hovered_index = Some(global_idx);
                                    cx.notify();
                                }
                            })
                            .ok();
                    })
                    .child(
                        div()
                            .when(kind == TileKind::Glyph, |this| this.text_lg())
                            .when(kind == TileKind::Pair, |this| this.text_xs())
                            .when(kind == TileKind::Abbr, |this| {
                                this.text_xs().font_semibold().text_color(pal.muted)
                            })
                            .text_color(pal.foreground)
                            .child(item.display),
                    );

                grid = grid.child(tile);
            }
            grid.into_any_element()
        };

        // 상세 안내 바
        let info_bar = if let Some(item) = active_item {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .p_2()
                .rounded_md()
                .bg(pal.panel)
                .border_1()
                .border_color(pal.panel_border)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_w(px(0.))
                        .child(
                            div()
                                .flex()
                                .flex_shrink_0()
                                .items_center()
                                .justify_center()
                                .size(px(30.0))
                                .rounded_md()
                                .bg(pal.hover)
                                .text_lg()
                                .child(item.display),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_semibold()
                                        .text_color(pal.foreground)
                                        .child(t!(item.name)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(pal.muted)
                                        .truncate()
                                        .child(t!(item.desc)),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .font_family(mono)
                        .text_xs()
                        .text_color(pal.muted)
                        .child(code_point(item)),
                )
        } else {
            div()
                .p_2()
                .rounded_md()
                .bg(pal.panel)
                .text_xs()
                .text_color(pal.muted)
                .child(t!("special_char.hint"))
        };

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(tabs)
            .child(Input::new(&self.search).focus_ring(false))
            .child(tiles)
            .child(info_bar)
    }
}

impl WriterApp {
    /// 특수 문자 및 이모지 삽입 대화상자를 연다. 열린 시트가 없으면 안내만 한다.
    pub(super) fn on_insert_special_char(
        &mut self,
        _: &InsertSpecialChar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = &self.editor else {
            self.notify(t!("common.open_sheet_first"), window, cx);
            return;
        };
        let target = editor.state().clone();
        let app = cx.entity().downgrade();
        let form = cx.new(|cx| SpecialCharForm::new(app, target, window, cx));
        let initial_focus = Cell::new(true);

        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                let search = form.read(cx).search.clone();
                search.update(cx, |state, cx| state.focus(window, cx));
            }
            // Enter(대화상자 확인)와 `삽입` 버튼은 같은 일을 한다: 지금 활성인 칸을 넣는다.
            let submit = {
                let form = form.clone();
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    form.update(cx, |form, cx| form.insert_active(window, cx))
                })
            };
            let (enter, ok) = (submit.clone(), submit.clone());
            dialog
                .w(px(DIALOG_WIDTH))
                .title(t!("special_char.title"))
                .close_button(true)
                .on_ok(move |_, window, cx| enter(window, cx))
                .child(form.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("insert-special-char-close")
                                .label(t!("common.close"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("insert-special-char-ok")
                                .primary()
                                .label(t!("common.insert"))
                                .on_click(move |_, window, cx| {
                                    if ok(window, cx) {
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
        });
    }

    /// 선택된 특수 문자 항목을 에디터에 삽입한다.
    pub(super) fn insert_special_char_item(
        &mut self,
        target: &Entity<EditorState>,
        item: &SpecialCharItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = &self.editor else { return };
        if editor.state() != target {
            self.notify(t!("special_char.sheet_closed"), window, cx);
            return;
        }

        editor.state().update(cx, |state, cx| {
            if item.is_pair {
                let text = state.value();
                let selected = state.selected_range();
                if selected.is_empty() {
                    let at = selected.start.min(text.len());
                    state.set_selected_range(at..at, cx);
                    state.replace(item.insert, window, cx);
                    let inside = at + item.pair_open_len;
                    state.set_selected_range(inside..inside, cx);
                } else {
                    let inside_text = &text[selected.clone()];
                    let open = &item.insert[..item.pair_open_len];
                    let close = &item.insert[item.pair_open_len..];
                    let wrapped = format!("{open}{inside_text}{close}");
                    let start = selected.start;
                    let end = start + wrapped.len();
                    state.replace(wrapped, window, cx);
                    state.set_selected_range(end..end, cx);
                }
            } else {
                state.replace(item.insert, window, cx);
            }
            state.focus(window, cx);
        });
        self.on_editor_changed(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{CharCategory, SPECIAL_CHARS, TileKind, code_point, tile_kind};

    #[test]
    fn test_special_chars_contain_typesetting_spaces_and_quotes() {
        let has_nbsp = SPECIAL_CHARS.iter().any(|item| item.insert == "\u{00A0}");
        let has_thin = SPECIAL_CHARS.iter().any(|item| item.insert == "\u{2009}");
        let has_em_dash = SPECIAL_CHARS.iter().any(|item| item.insert == "—");
        let has_natpyo = SPECIAL_CHARS
            .iter()
            .any(|item| item.insert == "「」" && item.is_pair);
        let has_gyeop_natpyo = SPECIAL_CHARS
            .iter()
            .any(|item| item.insert == "『』" && item.is_pair);

        assert!(has_nbsp, "NBSP space should exist");
        assert!(has_thin, "Thin space should exist");
        assert!(has_em_dash, "Em dash should exist");
        assert!(has_natpyo, "낫표 pair should exist");
        assert!(has_gyeop_natpyo, "겹낫표 pair should exist");
    }

    #[test]
    fn test_special_chars_categories() {
        let typesetting_count = SPECIAL_CHARS
            .iter()
            .filter(|i| i.category == CharCategory::Typesetting)
            .count();
        let quotes_count = SPECIAL_CHARS
            .iter()
            .filter(|i| i.category == CharCategory::Quotes)
            .count();
        let emoji_count = SPECIAL_CHARS
            .iter()
            .filter(|i| i.category == CharCategory::Emoji)
            .count();

        assert!(typesetting_count >= 15);
        assert!(quotes_count >= 15);
        assert!(emoji_count >= 20);
    }

    #[test]
    fn test_pair_open_len_validity() {
        for item in SPECIAL_CHARS.iter().filter(|i| i.is_pair) {
            assert!(
                item.pair_open_len > 0 && item.pair_open_len < item.insert.len(),
                "Pair item {} has invalid pair_open_len",
                item.display
            );
            assert!(
                item.insert.is_char_boundary(item.pair_open_len),
                "Pair item {} pair_open_len is not a char boundary",
                item.display
            );
        }
    }

    #[test]
    fn tile_kind_follows_visible_glyphs() {
        // 칸 크기는 바이트 수가 아니라 보이는 글자 수로 정한다(이모지는 4바이트지만 글자 하나).
        let kind_of = |display: &str| {
            SPECIAL_CHARS
                .iter()
                .find(|item| item.display == display)
                .map(tile_kind)
        };
        assert_eq!(kind_of("NBSP"), Some(TileKind::Abbr));
        assert_eq!(kind_of("ZWSP"), Some(TileKind::Abbr));
        assert_eq!(kind_of("📝"), Some(TileKind::Glyph));
        // 변이 선택자(U+FE0F)만 붙은 이모지도 글자 하나로 본다.
        assert_eq!(kind_of("🖋️"), Some(TileKind::Glyph));
        assert_eq!(kind_of("$"), Some(TileKind::Glyph));
        assert_eq!(kind_of("「…」"), Some(TileKind::Pair));
    }

    #[test]
    fn code_point_is_the_first_inserted_char() {
        let nbso = SPECIAL_CHARS
            .iter()
            .find(|item| item.insert == "\u{00A0}")
            .expect("NBSP 항목 없음");
        assert_eq!(code_point(nbso), "U+00A0");
        // 괄호 싸개는 여는 글자를 보여 준다.
        let pair = SPECIAL_CHARS
            .iter()
            .find(|item| item.is_pair)
            .expect("싸개 항목 없음");
        assert!(code_point(pair).starts_with("U+"));
    }
}
