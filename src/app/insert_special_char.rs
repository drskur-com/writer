//! 특수 문자 및 이모지 삽입 대화상자: 조판 공백, 대시, 낫표/겹낫표, 기호, 이모지 등을 선택해 삽입한다.

use gpui_base::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{EditorState, Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, FocusableExt as _, WindowExt as _};
use gpui_kit::gpui::prelude::{FluentBuilder as _, InteractiveElement as _, StatefulInteractiveElement as _};
use gpui_kit::gpui::*;
use std::cell::Cell;

use super::WriterApp;
use super::style::Palette;
use crate::InsertSpecialChar;

/// 대화상자 기본 너비.
const DIALOG_WIDTH: f32 = 560.0;

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

pub(crate) static SPECIAL_CHARS: &[SpecialCharItem] = &[
    // --- 조판 및 공백 (Typesetting & Spaces) ---
    item("NBSP", "\u{00A0}", "줄바꿈 없는 공백 (NBSP)", "단위나 숫자 뒤에서 줄이 나뉘지 않게 고정하는 공백 (U+00A0)", CharCategory::Typesetting, "공백 nbsp space 줄바꿈 non-breaking"),
    item("Thin", "\u{2009}", "얇은 공백 (Thin Space)", "따옴표 안쪽이나 기호 간 미세 간격 조정용 좁은 공백 (U+2009)", CharCategory::Typesetting, "공백 thin space 얇은"),
    item("En Sp", "\u{2002}", "엔 공백 (반각)", "일반 공백보다 약간 넓은 반각 너비의 공백 (U+2002)", CharCategory::Typesetting, "공백 en space 반각"),
    item("Em Sp", "\u{2003}", "엠 공백 (전각)", "한 글자 전각 너비의 공백 (U+2003)", CharCategory::Typesetting, "공백 em space 전각"),
    item("ZWSP", "\u{200B}", "너비 없는 공백 (ZWSP)", "보이지 않지만 긴 단어의 임의 줄바꿈 위치 지정 (U+200B)", CharCategory::Typesetting, "공백 zwsp zero width space 줄바꿈"),
    item("WJ", "\u{2060}", "결합 방지 (Word Joiner)", "앞뒤 글자가 절대 분리되어 줄바꿈되지 않게 방지 (U+2060)", CharCategory::Typesetting, "공백 결합방지 wj word joiner"),
    item("SHY", "\u{00AD}", "임의 하이픈 (Soft Hyphen)", "줄 끝에 걸칠 때만 붙임표로 분리되는 숨김 하이픈 (U+00AD)", CharCategory::Typesetting, "하이픈 shy soft hyphen 붙임표"),
    item("‑", "\u{2011}", "줄바꿈 없는 붙임표", "하이픈 위치에서 줄바꿈이 일어나지 않는 붙임표 (U+2011)", CharCategory::Typesetting, "하이픈 non-breaking hyphen 붙임표"),
    item("—", "—", "엠 대시 (Em Dash)", "생각의 전환이나 부연 설명을 나타내는 긴 줄표 (U+2014)", CharCategory::Typesetting, "대시 em dash 줄표"),
    item("–", "–", "엔 대시 (En Dash)", "기간, 범위(예: 1950–1953)를 나타내는 줄표 (U+2013)", CharCategory::Typesetting, "대시 en dash 줄표 기간 범위"),
    item("…", "…", "말줄임표 (삼점 리더)", "정식 마크다운/조판 말줄임표 (U+2026)", CharCategory::Typesetting, "말줄임표 ellipsis 점 리더"),
    item("‥", "‥", "이점 리더", "두 점으로 된 말줄임표 (U+2025)", CharCategory::Typesetting, "말줄임표 이점 점"),
    item("·", "·", "가운뎃점", "열거된 어구 사이를 구분하는 점 (U+00B7)", CharCategory::Typesetting, "가운뎃점 middle dot 점"),
    item("•", "•", "불릿 기호 (Bullet)", "글머리 기호 (U+2022)", CharCategory::Typesetting, "불릿 bullet 목록 점"),
    item("※", "※", "참고표", "주석 및 주의 사항 표시 (U+203B)", CharCategory::Typesetting, "참고표 당구장 주석"),
    item("¶", "¶", "단락 기호 (Pilcrow)", "문단 구분을 나타내는 기호 (U+00B6)", CharCategory::Typesetting, "단락 문단 pilcrow paragraph"),
    item("§", "§", "절 기호 (Section)", "서적의 절이나 법조항을 나타내는 기호 (U+00A7)", CharCategory::Typesetting, "절 섹션 section 조항"),
    item("†", "†", "칼표 (Dagger)", "각주나 사망 표기에 쓰는 기호 (U+2020)", CharCategory::Typesetting, "칼표 dagger 각주"),
    item("‡", "‡", "이중 칼표", "두 번째 각주에 쓰는 기호 (U+2021)", CharCategory::Typesetting, "칼표 double dagger 각주"),

    // --- 따옴표 및 괄호 (Quotes & Brackets) ---
    pair("「…」", "「」", "낫표 묶음", "단행본 속 소제목, 짧은 인용구, 예술작품 편명 감싸기", CharCategory::Quotes, "낫표 괄호 인용", 3),
    pair("『…』", "『』", "겹낫표 묶음", "책 제목, 신문/잡지 등 정기간행물 제목 감싸기", CharCategory::Quotes, "겹낫표 책제목 괄호 인용", 3),
    pair("〈…〉", "〈〉", "홑화살괄호 묶음", "소제목, 악곡, 예술 작품의 이름 감싸기", CharCategory::Quotes, "홑화살괄호 꺾쇠 괄호", 3),
    pair("《…》", "《》", "겹화살괄호 묶음", "책의 제목, 신문/잡지 등의 이름 감싸기", CharCategory::Quotes, "겹화살괄호 책제목 꺾쇠", 3),
    pair("“…”", "“”", "큰따옴표 묶음", "직접 대화나 인용문 감싸기", CharCategory::Quotes, "큰따옴표 quote 인용", 3),
    pair("‘…’", "‘’", "작은따옴표 묶음", "속마음이나 재인용문 감싸기", CharCategory::Quotes, "작은따옴표 quote 인용", 3),
    pair("«…»", "«»", "기예메 (프렌치 쿼트) 묶음", "유럽 조판 인용 부호 감싸기", CharCategory::Quotes, "기예메 guillemet 인용", 2),
    pair("【…】", "【】", "볼드 꺾쇠괄호 묶음", "표제어 및 강조 괄호 감싸기", CharCategory::Quotes, "볼드 꺾쇠 괄호", 3),
    pair("〔…〕", "〔〕", "거북등 괄호 묶음", "보충 설명 및 각주 부호 감싸기", CharCategory::Quotes, "거북등 괄호 귀호", 3),

    item("「", "「", "낫표 (열기)", "여는 낫표 (U+300C)", CharCategory::Quotes, "낫표 괄호 인용"),
    item("」", "」", "낫표 (닫기)", "닫는 낫표 (U+300D)", CharCategory::Quotes, "낫표 괄호 인용"),
    item("『", "『", "겹낫표 (열기)", "여는 겹낫표 (U+300E)", CharCategory::Quotes, "겹낫표 괄호 책제목"),
    item("』", "』", "겹낫표 (닫기)", "닫는 겹낫표 (U+300F)", CharCategory::Quotes, "겹낫표 괄호 책제목"),
    item("〈", "〈", "홑화살괄호 (열기)", "여는 홑화살괄호 (U+3008)", CharCategory::Quotes, "홑화살괄호 꺾쇠"),
    item("〉", "〉", "홑화살괄호 (닫기)", "닫는 홑화살괄호 (U+3009)", CharCategory::Quotes, "홑화살괄호 꺾쇠"),
    item("《", "《", "겹화살괄호 (열기)", "여는 겹화살괄호 (U+300A)", CharCategory::Quotes, "겹화살괄호 꺾쇠 책제목"),
    item("》", "》", "겹화살괄호 (닫기)", "닫는 겹화살괄호 (U+300B)", CharCategory::Quotes, "겹화살괄호 꺾쇠 책제목"),
    item("“", "“", "여는 큰따옴표", "굽은 큰따옴표 열기 (U+201C)", CharCategory::Quotes, "큰따옴표 quote 인용"),
    item("”", "”", "닫는 큰따옴표", "굽은 큰따옴표 닫기 (U+201D)", CharCategory::Quotes, "큰따옴표 quote 인용"),
    item("‘", "‘", "여는 작은따옴표", "굽은 작은따옴표 열기 (U+2018)", CharCategory::Quotes, "작은따옴표 quote 인용"),
    item("’", "’", "닫는 작은따옴표", "굽은 작은따옴표 닫기 (U+2019)", CharCategory::Quotes, "작은따옴표 quote 인용"),

    // --- 기호 및 단위 (Symbols & Units) ---
    item("©", "©", "저작권 (Copyright)", "저작권 표시 (U+00A9)", CharCategory::Symbols, "저작권 copyright copy c"),
    item("®", "®", "등록상표 (Registered)", "등록상표 기호 (U+00AE)", CharCategory::Symbols, "등록상표 registered r tm"),
    item("™", "™", "상표 (Trademark)", "일반 상표 기호 (U+2122)", CharCategory::Symbols, "상표 trademark tm"),
    item("°", "°", "도 (Degree)", "각도 및 온도를 나타내는 단위 (U+00B0)", CharCategory::Symbols, "도 degree 각도 온도"),
    item("′", "′", "분 / 프라임", "각도의 분, 피트 단위를 나타내는 기호 (U+2032)", CharCategory::Symbols, "프라임 분 prime foot"),
    item("″", "″", "초 / 더블 프라임", "각도의 초, 인치 단위를 나타내는 기호 (U+2033)", CharCategory::Symbols, "더블프라임 초 inch double prime"),
    item("℃", "℃", "섭씨 온도", "섭씨 기호 (U+2103)", CharCategory::Symbols, "온도 섭씨 celcius"),
    item("℉", "℉", "화씨 온도", "화씨 기호 (U+2109)", CharCategory::Symbols, "온도 화씨 fahrenheit"),
    item("‰", "‰", "퍼밀 (천분율)", "천분율 기호 (U+2030)", CharCategory::Symbols, "퍼밀 천분율 permil"),
    item("₩", "₩", "원화 (KRW)", "한국 원화 기호 (U+20A9)", CharCategory::Symbols, "원 통화 money won krw"),
    item("$", "$", "달러 (USD)", "달러 기호 (U+0024)", CharCategory::Symbols, "달러 통화 money dollar usd"),
    item("€", "€", "유로 (EUR)", "유로 기호 (U+20AC)", CharCategory::Symbols, "유로 통화 money euro eur"),
    item("£", "£", "파운드 (GBP)", "파운드 기호 (U+00A3)", CharCategory::Symbols, "파운드 통화 money pound gbp"),
    item("¥", "¥", "엔/위안 (JPY/CNY)", "엔화 및 위안화 기호 (U+00A5)", CharCategory::Symbols, "엔 위안 통화 yen yuan"),
    item("±", "±", "플러스마이너스", "오차 또는 양/음 기호 (U+00B1)", CharCategory::Symbols, "플러스마이너스 plusminus 오차"),
    item("×", "×", "곱셈 / 판형 치수", "곱셈 부호 또는 책 판형 크기 표기 (U+00D7)", CharCategory::Symbols, "곱셈 치수 크기 multiply"),
    item("÷", "÷", "나눗셈", "나눗셈 부호 (U+00F7)", CharCategory::Symbols, "나눗셈 divide"),
    item("≠", "≠", "같지 않음", "불일치 관계 기호 (U+2260)", CharCategory::Symbols, "같지않음 다름 not equal"),
    item("≤", "≤", "작거나 같음", "부등호 (U+2264)", CharCategory::Symbols, "작거나같음 이하 less equal"),
    item("≥", "≥", "크거나 같음", "부등호 (U+2265)", CharCategory::Symbols, "크거나같음 이상 greater equal"),
    item("≈", "≈", "근사 (거의 같음)", "근사치 기호 (U+2248)", CharCategory::Symbols, "근사 약 approx"),
    item("∞", "∞", "무한대", "무한 기호 (U+221E)", CharCategory::Symbols, "무한대 infinity"),
    item("√", "√", "제곱근 (루트)", "루트 기호 (U+221A)", CharCategory::Symbols, "루트 root"),
    item("←", "←", "왼쪽 화살표", "왼쪽 방향 (U+2190)", CharCategory::Symbols, "화살표 arrow left"),
    item("→", "→", "오른쪽 화살표", "오른쪽 방향 (U+2192)", CharCategory::Symbols, "화살표 arrow right"),
    item("↑", "↑", "위쪽 화살표", "위쪽 방향 (U+2191)", CharCategory::Symbols, "화살표 arrow up"),
    item("↓", "↓", "아래쪽 화살표", "아래쪽 방향 (U+2193)", CharCategory::Symbols, "화살표 arrow down"),
    item("↔", "↔", "양방향 화살표", "양방향 (U+2194)", CharCategory::Symbols, "화살표 arrow both"),
    item("⇒", "⇒", "귀결 화살표", "도출 및 결과 표시 (U+21D2)", CharCategory::Symbols, "화살표 implies arrow"),
    item("✓", "✓", "체크 표시", "체크 마크 (U+2713)", CharCategory::Symbols, "체크 check mark"),
    item("★", "★", "검은 별", "채운 별표 (U+2605)", CharCategory::Symbols, "별 star"),
    item("☆", "☆", "빈 별", "빈 별표 (U+2606)", CharCategory::Symbols, "별 star"),
    item("◆", "◆", "검은 다이아몬드", "채운 다이아몬드 (U+25C6)", CharCategory::Symbols, "다이아몬드 diamond"),
    item("◇", "◇", "빈 다이아몬드", "빈 다이아몬드 (U+25C7)", CharCategory::Symbols, "다이아몬드 diamond"),
    item("■", "■", "검은 네모", "채운 사각형 (U+25A0)", CharCategory::Symbols, "네모 square"),
    item("□", "□", "빈 네모", "빈 사각형 (U+25A1)", CharCategory::Symbols, "네모 square"),

    // --- 이모지 (Emoji) ---
    item("📝", "📝", "메모", "글쓰기 메모 (U+1F4DD)", CharCategory::Emoji, "메모 memo note 글쓰기"),
    item("📖", "📖", "펼친 책", "열린 책 (U+1F4D6)", CharCategory::Emoji, "책 book read 독서"),
    item("📚", "📚", "책장 / 서적", "책 더미 (U+1F4DA)", CharCategory::Emoji, "책 서적 books library"),
    item("📄", "📄", "문서", "문서 페이지 (U+1F4C4)", CharCategory::Emoji, "문서 doc page paper"),
    item("🖋️", "🖋️", "만년필", "만년필 펜 (U+1F58B)", CharCategory::Emoji, "펜 만년필 pen fountain"),
    item("✏️", "✏️", "연필", "연필 (U+270F)", CharCategory::Emoji, "연필 pencil write"),
    item("📌", "📌", "압정", "압정 핀 (U+1F4CC)", CharCategory::Emoji, "압정 pin 고정"),
    item("📍", "📍", "둥근 핀", "위치 표시 핀 (U+1F4CD)", CharCategory::Emoji, "핀 pin 위치"),
    item("📎", "📎", "클립", "종이 클립 (U+1F4CE)", CharCategory::Emoji, "클립 clip 첨부"),
    item("🔖", "🔖", "책갈피", "북마크 책갈피 (U+1F516)", CharCategory::Emoji, "책갈피 bookmark 북마크"),
    item("💡", "💡", "전구 / 아이디어", "아이디어 발상 (U+1F4A1)", CharCategory::Emoji, "전구 아이디어 idea lightbulb"),
    item("💭", "💭", "생각 풍선", "생각 버블 (U+1F4AD)", CharCategory::Emoji, "생각 thought bubble"),
    item("💬", "💬", "말풍선", "대화 말풍선 (U+1F4AC)", CharCategory::Emoji, "말풍선 comment speech chat"),
    item("✍️", "✍️", "글쓰는 손", "집필 및 서명 (U+270D)", CharCategory::Emoji, "글쓰기 write hand 서명"),
    item("🗓️", "🗓️", "달력", "일정 및 날짜 (U+1F5D3)", CharCategory::Emoji, "달력 calendar 날짜"),
    item("⏱️", "⏱️", "스톱워치", "시간 측정 (U+23F1)", CharCategory::Emoji, "시간 timer stopwatch 시계"),
    item("⏳", "⏳", "모래시계", "진행 중인 시간 (U+23F3)", CharCategory::Emoji, "모래시계 hourglass 시간"),
    item("😀", "😀", "활짝 웃는 얼굴", "웃음 (U+1F600)", CharCategory::Emoji, "웃음 smile grin"),
    item("😊", "😊", "미소 짓는 얼굴", "따뜻한 미소 (U+1F60A)", CharCategory::Emoji, "미소 smile happy"),
    item("🥰", "🥰", "하트 얼굴", "애정 어린 표정 (U+1F970)", CharCategory::Emoji, "하트 love heart"),
    item("😍", "😍", "반한 얼굴", "반한 눈빛 (U+1F60D)", CharCategory::Emoji, "하트 반함 love heart eyes"),
    item("🤔", "🤔", "고민하는 얼굴", "생각 중인 표정 (U+1F914)", CharCategory::Emoji, "생각 고민 thinking think"),
    item("🧐", "🧐", "단안경 쓴 얼굴", "주의 깊게 관찰 (U+1F9D0)", CharCategory::Emoji, "관찰 monocle inspect"),
    item("🤫", "🤫", "쉿 하는 얼굴", "조용히 (U+1F92B)", CharCategory::Emoji, "쉿 quiet shh"),
    item("😅", "😅", "식은땀 흘리는 얼굴", "안도 또는 당황 (U+1F605)", CharCategory::Emoji, "땀 sweat sweat smile"),
    item("😢", "😢", "눈물 흘리는 얼굴", "슬픔 (U+1F622)", CharCategory::Emoji, "눈물 슬픔 cry sad"),
    item("😭", "😭", "엉엉 우는 얼굴", "대성통곡 (U+1F62D)", CharCategory::Emoji, "눈물 슬픔 sob crying"),
    item("🥳", "🥳", "축하 파티 얼굴", "축하 (U+1F973)", CharCategory::Emoji, "파티 축하 party celebrate"),
    item("😎", "😎", "선글라스 낀 얼굴", "자신감 (U+1F60E)", CharCategory::Emoji, "선글라스 cool sunglasses"),
    item("🤩", "🤩", "눈이 반짝이는 얼굴", "환호와 감탄 (U+1F929)", CharCategory::Emoji, "스타 star eyes 반짝"),
    item("👍", "👍", "엄지 척 (좋아요)", "승인 및 추천 (U+1F44D)", CharCategory::Emoji, "좋아요 thumbs up good"),
    item("👎", "👎", "엄지 아래 (싫어요)", "비추천 (U+1F44E)", CharCategory::Emoji, "싫어요 thumbs down bad"),
    item("👏", "👏", "박수", "격려와 박수 (U+1F44F)", CharCategory::Emoji, "박수 clap applause"),
    item("🙌", "🙌", "만세", "환호 (U+1F64C)", CharCategory::Emoji, "만세 celebrate hooray"),
    item("🤝", "🤝", "악수", "합의와 제휴 (U+1F91D)", CharCategory::Emoji, "악수 handshake deal"),
    item("✌️", "✌️", "브이 (승리)", "평화와 승리 (U+270C)", CharCategory::Emoji, "브이 victory peace"),
    item("🤞", "🤞", "행운을 비는 손가락", "기원 (U+1F91E)", CharCategory::Emoji, "행운 luck fingers crossed"),
    item("✅", "✅", "체크 마크", "완료 및 승인 (U+2705)", CharCategory::Emoji, "체크 완료 check done"),
    item("❌", "❌", "엑스 마크", "취소 및 오류 (U+274C)", CharCategory::Emoji, "엑스 취소 cross x no"),
    item("⚠️", "⚠️", "경고", "주의 및 경고 (U+26A0)", CharCategory::Emoji, "경고 warning caution"),
    item("ℹ️", "ℹ️", "정보", "안내 정보 (U+2139)", CharCategory::Emoji, "정보 info information"),
    item("❓", "❓", "물음표", "질문 (U+2753)", CharCategory::Emoji, "물음표 question mark"),
    item("❗", "❗", "느낌표", "강조 (U+2757)", CharCategory::Emoji, "느낌표 exclamation mark"),
    item("🔴", "🔴", "빨간 원", "빨간색 표시 (U+1F534)", CharCategory::Emoji, "빨강 red circle"),
    item("🟢", "🟢", "초록 원", "초록색 표시 (U+1F7E2)", CharCategory::Emoji, "초록 green circle"),
    item("🔵", "🔵", "파란 원", "파란색 표시 (U+1F535)", CharCategory::Emoji, "파랑 blue circle"),
    item("⚪", "⚪", "흰 원", "흰색 표시 (U+26AA)", CharCategory::Emoji, "흰색 white circle"),
    item("⚫", "⚫", "검은 원", "검은색 표시 (U+26AB)", CharCategory::Emoji, "검정 black circle"),
    item("🚀", "🚀", "로켓", "발사 및 도약 (U+1F680)", CharCategory::Emoji, "로켓 rocket launch"),
    item("✨", "✨", "반짝이", "빛남과 강조 (U+2728)", CharCategory::Emoji, "반짝 sparkles star"),
    item("🎉", "🎉", "폭죽", "축하 (U+1F389)", CharCategory::Emoji, "파티 폭죽 party tada"),
    item("☕", "☕", "커피", "휴식과 집필 (U+2615)", CharCategory::Emoji, "커피 coffee break"),
    item("🍵", "🍵", "차", "따뜻한 차 (U+1F375)", CharCategory::Emoji, "차 tea green tea"),
    item("🌿", "🌿", "허브 / 새싹", "자연과 생기 (U+1F33F)", CharCategory::Emoji, "새싹 허브 herb plant"),
    item("🔥", "🔥", "불꽃", "열정과 인기 (U+1F525)", CharCategory::Emoji, "불 fire flame hot"),
    item("⭐️", "⭐️", "별", "평점 및 중요 (U+2B50)", CharCategory::Emoji, "별 star rate"),
    item("💯", "💯", "백점 만점", "완벽함 (U+1F4AF)", CharCategory::Emoji, "백점 100 hundred perfect"),
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
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("특수 문자 검색… (예: 공백, 낫표, 대시, em, smile)"));
        let search_sub = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let enter_sub = cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.submit_first(window, cx);
            }
        });

        Self {
            app,
            target,
            search,
            category: CharCategory::All,
            hovered_index: None,
            _subscriptions: vec![search_sub, enter_sub],
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
                    || item.name.to_lowercase().contains(&query)
                    || item.desc.to_lowercase().contains(&query)
                    || item.keywords.to_lowercase().contains(&query)
            })
            .collect()
    }

    fn submit_first(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.filtered_items(cx);
        if let Some((_, item)) = items.first() {
            let item = **item;
            self.insert_item(&item, window, cx);
            window.close_dialog(cx);
        }
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
            (CharCategory::All, "전체"),
            (CharCategory::Typesetting, "조판·공백"),
            (CharCategory::Quotes, "따옴표·괄호"),
            (CharCategory::Symbols, "기호·단위"),
            (CharCategory::Emoji, "이모지"),
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
            .children(categories.iter().map(|(_, label)| Tab::new().flex_1().label(*label)));

        let filtered = self.filtered_items(cx);
        let active_item: Option<&'static SpecialCharItem> = self
            .hovered_index
            .and_then(|idx| SPECIAL_CHARS.get(idx))
            .or_else(|| filtered.first().map(|(_, item)| *item));

        let tiles = if filtered.is_empty() {
            div()
                .flex()
                .items_center()
                .justify_center()
                .h(px(220.0))
                .text_sm()
                .text_color(pal.muted)
                .child("일치하는 특수 문자가 없습니다.")
                .into_any_element()
        } else {
            let mut grid = div()
                .flex()
                .flex_wrap()
                .gap_1_5()
                .p_1()
                .max_h(px(240.0))
                .overflow_y_scrollbar();

            for (global_idx, item) in filtered {
                let is_badge = item.display.len() > 3;
                let form_for_click = cx.entity().downgrade();
                let form_for_hover = cx.entity().downgrade();
                let item_copy = *item;
                let name = item.name;
                let desc = item.desc;

                let tile = div()
                    .id(("char-tile", global_idx))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .rounded_md()
                    .border_1()
                    .border_color(pal.border)
                    .bg(pal.panel)
                    .hover(|s| s.bg(pal.hover).border_color(pal.tint_foreground))
                    .when(is_badge, |this| this.px_2().h(px(34.0)))
                    .when(!is_badge, |this| this.size(px(34.0)))
                    .tooltip(move |window, cx| Tooltip::new(format!("{name}\n{desc}")).build(window, cx))
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
                            .when(is_badge, |this| this.text_xs().font_semibold())
                            .when(!is_badge, |this| this.text_base())
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
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size(px(32.0))
                                .rounded_md()
                                .bg(pal.hover)
                                .text_base()
                                .font_bold()
                                .child(item.display),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_semibold()
                                        .text_color(pal.foreground)
                                        .child(item.name),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(pal.muted)
                                        .child(item.desc),
                                ),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(pal.muted)
                        .child("클릭하여 삽입"),
                )
        } else {
            div()
                .p_2()
                .rounded_md()
                .bg(pal.panel)
                .text_xs()
                .text_color(pal.muted)
                .child("원하는 문자를 클릭하면 커서 자리에 삽입됩니다.")
        };

        // 단축키 및 시스템 패널 안내
        #[cfg(target_os = "macos")]
        let shortcut_note = "💡 단축키: ⌃⌘Space (macOS 문자 뷰어는 Edit 메뉴에서도 지원됩니다)";
        #[cfg(not(target_os = "macos"))]
        let shortcut_note = "💡 단축키: Ctrl+Alt+Space (Windows 시스템 이모지: Win + .)";

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(tabs)
            .child(Input::new(&self.search).focus_ring(false))
            .child(tiles)
            .child(info_bar)
            .child(
                div()
                    .text_xs()
                    .text_color(pal.muted)
                    .child(shortcut_note),
            )
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
            self.notify("먼저 시트를 여세요.", window, cx);
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
            dialog
                .w(px(DIALOG_WIDTH))
                .title("특수 문자 및 이모지")
                .close_button(true)
                .child(form.clone())
                .footer(
                    DialogFooter::new().child(
                        Button::new("insert-special-char-close")
                            .label("닫기")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
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
            self.notify("문자를 넣을 시트가 닫혔습니다.", window, cx);
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
    use super::{CharCategory, SPECIAL_CHARS};

    #[test]
    fn test_special_chars_contain_typesetting_spaces_and_quotes() {
        let has_nbsp = SPECIAL_CHARS.iter().any(|item| item.insert == "\u{00A0}");
        let has_thin = SPECIAL_CHARS.iter().any(|item| item.insert == "\u{2009}");
        let has_em_dash = SPECIAL_CHARS.iter().any(|item| item.insert == "—");
        let has_natpyo = SPECIAL_CHARS.iter().any(|item| item.insert == "「」" && item.is_pair);
        let has_gyeop_natpyo = SPECIAL_CHARS.iter().any(|item| item.insert == "『』" && item.is_pair);

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
}
