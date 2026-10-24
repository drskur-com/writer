//! 이미지의 글자 인식(OCR, ADR-0053).
//!
//! 운영체제의 온디바이스 문자 인식으로 줄 단위 글자와 위치를 얻는다. 네트워크·모델 파일이 없다.
//! - macOS: Vision `VNRecognizeTextRequest`(정확 모드, 한국어·영어, 언어 보정).
//! - Windows: 쓸 수 있으면 Windows AI `TextRecognizer`(Copilot+ PC, [`text_recognizer`]), 아니면
//!   `Windows.Media.Ocr`(한국어 OCR 언어 팩이 있으면 한국어, 없으면 사용자 언어). 한국어 엔진은 라틴 문자도
//!   읽는다. 언어 팩은 Windows 설정의 언어 옵션에서 설치한다.
//!
//! 다른 운영체제에서는 컴파일하지 않는다.
//!
//! 에디터는 줄 하나가 문단이므로(ADR-0011) 인식한 줄을 그대로 넣으면 이미지의 줄바꿈마다 문단이
//! 나뉜다. 그래서 줄 사이 간격과 줄 길이로 문단을 다시 묶는다(`join`).

use std::path::Path;

/// 인식한 줄 하나. 좌표는 이미지 크기에 대한 비율이고 원점은 왼쪽 위다.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl Line {
    fn right(&self) -> f64 {
        self.left + self.width
    }

    fn bottom(&self) -> f64 {
        self.top + self.height
    }
}

/// 글자를 읽을 수 있는 이미지 확장자(ImageIO가 여는 형식).
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "heic", "heif", "tif", "tiff", "bmp", "webp",
];

/// `path`가 글자를 읽을 수 있는 이미지인지(확장자로 본다).
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .is_some_and(|ext| IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

/// `path` 이미지의 글자를 줄 단위로 읽는다. 읽는 동안 부른 스레드를 막으므로 백그라운드에서 부른다.
pub fn recognize(path: &Path) -> Result<Vec<Line>, String> {
    #[cfg(target_os = "macos")]
    return macos::recognize(path);
    #[cfg(target_os = "windows")]
    return windows_ocr::recognize(path);
}

/// 문단 끝으로 보는 줄 끝 글자(문장 부호·닫는 따옴표·괄호).
const TERMINALS: &[char] = &[
    '.', '?', '!', '。', '…', ':', '"', '\'', '”', '’', ')', '」', '』',
];

/// 인식한 줄을 에디터 텍스트로 잇는다. `keep_breaks`면 줄마다 한 줄(문단)로 둔다.
///
/// 그렇지 않으면 다음 줄이 이어지는 줄을 공백으로 붙여 문단으로 묶는다. 이런 때는 문단을 나눈다.
/// - 다음 줄과의 간격이 줄 높이의 0.8배보다 크다(빈 줄·문단 간격).
/// - 다음 줄이 위로 올라간다(다음 단·다른 영역).
/// - 줄이 글 영역 오른쪽 끝보다 눈에 띄게 짧고 문장 부호로 끝난다(문단의 마지막 줄).
///
/// 영어 낱말이 줄 끝 `-`로 나뉘었으면 하이픈을 빼고 붙인다.
pub fn join(lines: &[Line], keep_breaks: bool) -> String {
    let lines: Vec<&Line> = lines
        .iter()
        .filter(|line| !line.text.trim().is_empty())
        .collect();
    if keep_breaks {
        return lines
            .iter()
            .map(|line| line.text.trim())
            .collect::<Vec<_>>()
            .join("\n");
    }
    let Some(first) = lines.first() else {
        return String::new();
    };
    let mut heights: Vec<f64> = lines.iter().map(|line| line.height).collect();
    heights.sort_by(f64::total_cmp);
    let height = heights[heights.len() / 2];
    let left = lines.iter().map(|line| line.left).fold(f64::MAX, f64::min);
    let right = lines
        .iter()
        .map(|line| line.right())
        .fold(f64::MIN, f64::max);
    let short = right - (right - left) * 0.1;

    let mut out = first.text.trim().to_string();
    for pair in lines.windows(2) {
        let (prev, next) = (pair[0], pair[1]);
        let text = next.text.trim();
        let gap = next.top - prev.bottom();
        let ends_sentence = prev.text.trim_end().ends_with(TERMINALS);
        let breaks = gap > height * 0.8
            || next.top < prev.top - height * 0.5
            || (prev.right() < short && ends_sentence);
        if breaks {
            out.push('\n');
            out.push_str(text);
            continue;
        }
        let hyphenated = out.ends_with('-')
            && out[..out.len() - 1]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphabetic())
            && text.starts_with(|c: char| c.is_ascii_lowercase());
        if hyphenated {
            out.pop();
        } else {
            out.push(' ');
        }
        out.push_str(text);
    }
    out
}

#[cfg(target_os = "macos")]
mod macos {
    use super::Line;
    use objc2::AllocAnyThread as _;
    use objc2::rc::autoreleasepool;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};
    use objc2_vision::{
        VNImageOption, VNImageRequestHandler, VNRecognizeTextRequest, VNRequest,
        VNRequestTextRecognitionLevel,
    };
    use std::path::Path;

    /// 인식 언어(우선순위 순). 한국어는 macOS 13부터 된다.
    const LANGUAGES: [&str; 2] = ["ko-KR", "en-US"];

    pub(super) fn recognize(path: &Path) -> Result<Vec<Line>, String> {
        autoreleasepool(|_| {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            let options = NSDictionary::<VNImageOption, AnyObject>::new();
            let handler = unsafe {
                VNImageRequestHandler::initWithURL_options(
                    VNImageRequestHandler::alloc(),
                    &url,
                    &options,
                )
            };
            let request = VNRecognizeTextRequest::new();
            request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
            request.setUsesLanguageCorrection(true);
            let languages: Vec<_> = LANGUAGES.iter().map(|l| NSString::from_str(l)).collect();
            request.setRecognitionLanguages(&NSArray::from_retained_slice(&languages));
            let as_request: &VNRequest = &request;
            handler
                .performRequests_error(&NSArray::from_slice(&[as_request]))
                .map_err(|err| err.localizedDescription().to_string())?;
            let Some(results) = request.results() else {
                return Ok(Vec::new());
            };
            let mut lines = Vec::new();
            for index in 0..results.count() {
                let observation = results.objectAtIndex(index);
                let Some(text) = observation.topCandidates(1).firstObject() else {
                    continue;
                };
                // Vision의 상자는 원점이 왼쪽 아래다.
                let rect = unsafe { observation.boundingBox() };
                lines.push(Line {
                    text: text.string().to_string(),
                    left: rect.origin.x,
                    top: 1.0 - (rect.origin.y + rect.size.height),
                    width: rect.size.width,
                    height: rect.size.height,
                });
            }
            Ok(lines)
        })
    }
}

#[cfg(target_os = "windows")]
mod text_recognizer;

#[cfg(target_os = "windows")]
mod windows_ocr {
    use super::Line;
    use std::path::Path;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
        ExifOrientationMode,
    };
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::{FileAccessMode, StorageFile};
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::HSTRING;

    /// 먼저 쓰는 인식 언어. 언어 팩이 없으면 사용자 언어로 읽는다.
    const LANGUAGE: &str = "ko";

    pub(super) fn recognize(path: &Path) -> Result<Vec<Line>, String> {
        // WinRT를 부르는 스레드(백그라운드 실행기)를 MTA에 넣는다. 이미 들어가 있으면 결과는 무시한다.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        read(path).map_err(|err| err.message())
    }

    /// 쓸 인식 엔진. 언어 팩이 하나도 없으면 오류.
    fn engine() -> windows::core::Result<OcrEngine> {
        let preferred = Language::CreateLanguage(&HSTRING::from(LANGUAGE))
            .ok()
            .filter(|language| OcrEngine::IsLanguageSupported(language).unwrap_or(false))
            .and_then(|language| OcrEngine::TryCreateFromLanguage(&language).ok());
        match preferred {
            Some(engine) => Ok(engine),
            None => OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| {
                windows::core::Error::new(
                    windows::core::HRESULT(0x8000_4005_u32 as i32),
                    &t!("insert.ocr.windows_no_language"),
                )
            }),
        }
    }

    fn read(path: &Path) -> windows::core::Result<Vec<Line>> {
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.join()?;
        let stream = file.OpenAsync(FileAccessMode::Read)?.join()?;
        let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
        // 사진의 EXIF 방향대로 돌려서 읽는다.
        let bitmap = decoder
            .GetSoftwareBitmapTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Premultiplied,
                &BitmapTransform::new()?,
                ExifOrientationMode::RespectExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )?
            .join()?;
        // 쓸 수 있으면 Windows AI `TextRecognizer`(NPU)로 읽는다.
        if let Some(lines) = super::text_recognizer::recognize(&bitmap) {
            return Ok(lines);
        }
        let engine = engine()?;
        let limit = OcrEngine::MaxImageDimension()?;
        if bitmap.PixelWidth()?.max(bitmap.PixelHeight()?) as u32 > limit {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(0x8007_0057_u32 as i32),
                &t!("insert.ocr.windows_too_large", max = limit),
            ));
        }
        let (width, height) = (
            f64::from(bitmap.PixelWidth()?.max(1)),
            f64::from(bitmap.PixelHeight()?.max(1)),
        );
        let result = engine.RecognizeAsync(&bitmap)?.join()?;
        let mut lines = Vec::new();
        for line in &result.Lines()? {
            // 줄의 상자는 없어서 낱말 상자를 합친다.
            let (mut left, mut top, mut right, mut bottom) = (f64::MAX, f64::MAX, 0.0_f64, 0.0_f64);
            for word in &line.Words()? {
                let rect = word.BoundingRect()?;
                left = left.min(f64::from(rect.X));
                top = top.min(f64::from(rect.Y));
                right = right.max(f64::from(rect.X + rect.Width));
                bottom = bottom.max(f64::from(rect.Y + rect.Height));
            }
            if right <= left || bottom <= top {
                continue;
            }
            lines.push(Line {
                text: line.Text()?.to_string(),
                left: left / width,
                top: top / height,
                width: (right - left) / width,
                height: (bottom - top) / height,
            });
        }
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::{Line, join};

    fn line(text: &str, top: f64, width: f64) -> Line {
        Line {
            text: text.into(),
            left: 0.1,
            top,
            width,
            height: 0.03,
        }
    }

    #[test]
    fn wrapped_lines_become_one_paragraph() {
        let lines = [
            line("The quick brown fox jumps", 0.10, 0.8),
            line("over the lazy dog.", 0.14, 0.5),
        ];
        assert_eq!(
            join(&lines, false),
            "The quick brown fox jumps over the lazy dog."
        );
        assert_eq!(
            join(&lines, true),
            "The quick brown fox jumps\nover the lazy dog."
        );
    }

    #[test]
    fn short_line_ending_a_sentence_ends_the_paragraph() {
        let lines = [
            line("첫 문단의 첫 줄입니다 그리고", 0.10, 0.8),
            line("끝났습니다.", 0.14, 0.3),
            line("둘째 문단입니다.", 0.18, 0.4),
        ];
        assert_eq!(
            join(&lines, false),
            "첫 문단의 첫 줄입니다 그리고 끝났습니다.\n둘째 문단입니다."
        );
    }

    #[test]
    fn wide_gap_ends_the_paragraph() {
        let lines = [line("Title", 0.05, 0.8), line("Body text", 0.15, 0.8)];
        assert_eq!(join(&lines, false), "Title\nBody text");
    }

    #[test]
    fn hyphenated_word_is_rejoined() {
        let lines = [
            line("an extra-ordinary exam-", 0.10, 0.8),
            line("ple of text", 0.14, 0.8),
        ];
        assert_eq!(join(&lines, false), "an extra-ordinary example of text");
    }

    #[test]
    fn empty_lines_are_ignored() {
        assert_eq!(join(&[], false), "");
        assert_eq!(join(&[line("  ", 0.1, 0.5)], false), "");
    }
}
