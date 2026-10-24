//! Windows AI `TextRecognizer`(Windows App SDK, ADR-0053). Copilot+ PC의 NPU에서 돌고 `Windows.Media.Ocr`보다
//! 정확하다. 쓸 수 없으면 `None`을 돌려 부른 쪽이 `Windows.Media.Ocr`로 읽게 한다.
//!
//! 쓰려면 다음이 다 있어야 한다. 하나라도 없으면 조용히 넘어간다.
//! - Windows App SDK 2.5 런타임. 패키지 ID가 없는 exe는 실행 파일 옆의 `Microsoft.WindowsAppRuntime.Bootstrap.dll`
//!   (NuGet `Microsoft.WindowsAppSDK.Foundation`의 `runtimes/<아키텍처>/native/`)로 런타임을 프로세스에 붙인다.
//! - `systemAIModels` 권한이 든 패키지 ID(MSIX 또는 sparse package). 없으면 `GetReadyState`가 `Ready`가 아니다.
//! - 지원하는 하드웨어와 받아 둔 모델(`Ready`). `NotReady`(모델을 받아야 함)면 동의 없이 받지 않고 넘어간다.

#[allow(clippy::all, unused)]
mod bindings;

use super::Line;
use bindings::{AIFeatureReadyState, ImageBuffer, TextRecognizer};
use std::sync::OnceLock;
use windows::Graphics::Imaging::SoftwareBitmap;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{HRESULT, PCWSTR, s, w};

/// Windows App SDK 릴리스(주·부 버전, `WINDOWSAPPSDK_RELEASE_MAJORMINOR`).
const RELEASE_MAJOR_MINOR: u32 = 0x0002_0005;

/// 받아들이는 가장 낮은 런타임 버전(`WINDOWSAPPSDK_RUNTIME_VERSION_UINT64`, 2.5.1.0).
const MIN_RUNTIME_VERSION: u64 = 0x0002_0005_0001_0000;

/// 프로세스에 패키지 ID가 있으면 아무것도 하지 않는다(`MddBootstrapInitializeOptions_OnPackageIdentity_NOOP`).
const ON_PACKAGE_IDENTITY_NOOP: i32 = 0x0010;

/// `PACKAGE_VERSION`(값으로 넘기는 64비트 공용체).
#[repr(C)]
#[derive(Clone, Copy)]
struct PackageVersion {
    version: u64,
}

/// `MddBootstrapInitialize2(majorMinorVersion, versionTag, minVersion, options)`.
type MddBootstrapInitialize2 =
    unsafe extern "system" fn(u32, PCWSTR, PackageVersion, i32) -> HRESULT;

/// 이 프로세스에서 Windows App SDK 런타임을 쓸 수 있는지. 처음 한 번만 붙여 본다.
fn runtime_ready() -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    *READY.get_or_init(|| unsafe {
        let Ok(module) = LoadLibraryW(w!("Microsoft.WindowsAppRuntime.Bootstrap.dll")) else {
            return false;
        };
        let Some(initialize) = GetProcAddress(module, s!("MddBootstrapInitialize2")) else {
            return false;
        };
        let initialize: MddBootstrapInitialize2 = std::mem::transmute(initialize);
        // 프로세스가 끝날 때까지 붙여 둔다(`MddBootstrapShutdown`을 부르지 않는다).
        initialize(
            RELEASE_MAJOR_MINOR,
            w!(""),
            PackageVersion {
                version: MIN_RUNTIME_VERSION,
            },
            ON_PACKAGE_IDENTITY_NOOP,
        )
        .is_ok()
    })
}

/// `bitmap`(이미 EXIF 방향대로 돌린 것)의 글자를 줄 단위로 읽는다. `TextRecognizer`를 쓸 수 없거나 읽다가
/// 실패하면 `None`이다.
pub(super) fn recognize(bitmap: &SoftwareBitmap) -> Option<Vec<Line>> {
    if !runtime_ready() {
        return None;
    }
    // 런타임은 있어도 클래스를 못 찾거나(활성화 실패) 권한이 없으면 오류가 난다.
    if TextRecognizer::GetReadyState().ok()? != AIFeatureReadyState::Ready {
        return None;
    }
    read(bitmap).ok()
}

fn read(bitmap: &SoftwareBitmap) -> windows::core::Result<Vec<Line>> {
    let width = f64::from(bitmap.PixelWidth()?.max(1));
    let height = f64::from(bitmap.PixelHeight()?.max(1));
    let recognizer = TextRecognizer::CreateAsync()?.join()?;
    let buffer = ImageBuffer::CreateForSoftwareBitmap(bitmap)?;
    let text = recognizer.RecognizeTextFromImage(&buffer)?;
    let mut lines = Vec::new();
    for line in text.Lines()?.iter().flatten() {
        // 상자는 기울 수 있는 사각형(네 꼭짓점, px)이다. 감싸는 수평 사각형으로 바꾼다.
        let bounds = line.BoundingBox()?;
        let points = [
            bounds.TopLeft,
            bounds.TopRight,
            bounds.BottomRight,
            bounds.BottomLeft,
        ];
        let xs = points.map(|point| f64::from(point.X));
        let ys = points.map(|point| f64::from(point.Y));
        let left = xs.into_iter().fold(f64::MAX, f64::min);
        let right = xs.into_iter().fold(f64::MIN, f64::max);
        let top = ys.into_iter().fold(f64::MAX, f64::min);
        let bottom = ys.into_iter().fold(f64::MIN, f64::max);
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
    recognizer.Close().ok();
    Ok(lines)
}
