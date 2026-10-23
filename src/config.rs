//! 앱 설정(머신 로컬 상태) 로드/저장.
//!
//! 라이브러리(사용자 데이터)와 분리해 OS 설정 디렉터리에 둔다.
//! - Windows: `%APPDATA%\Writer\config.toml`
//! - macOS: `~/Library/Application Support/Writer/config.toml`
//! - Linux: `~/.config/Writer/config.toml`

use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

/// 최근 라이브러리 목록 최대 개수.
const RECENT_LIMIT: usize = 5;

/// `config.toml` 스키마.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 현재 라이브러리 루트. 없으면 첫 실행(온보딩).
    pub library_root: Option<PathBuf>,
    /// 최근에 연 라이브러리(최신이 앞).
    pub recent_libraries: Vec<PathBuf>,
    /// 마지막으로 선택한 테마 이름. 없으면 기본 라이트.
    pub theme: Option<String>,
    /// 에디터 서체 이름. 없으면 시스템 서체.
    pub editor_font: Option<String>,
    /// 에디터 본문 글자 크기(px). 없으면 16.
    pub editor_font_size: Option<u16>,
    /// 어시스턴트가 쓸 AI 제공자(`bedrock`). 없으면 쓰지 않는다.
    pub ai_provider: Option<String>,
    /// Bedrock에 쓸 AWS 프로필 이름(`~/.aws/config`). 없으면 고르지 않은 것.
    pub aws_profile: Option<String>,
    /// Bedrock을 부를 리전. 없으면 프로필의 리전을 따른다.
    pub aws_region: Option<String>,
    /// 어시스턴트가 부를 Bedrock 모델(추론 프로필) ID. 없으면 `agent::DEFAULT_MODEL`.
    pub ai_model: Option<String>,
    /// UI 언어(`en`·`ko`). 없으면 시스템 언어를 따른다.
    pub language: Option<String>,
}

/// 설정 디렉터리 (`<config_dir>/Writer`).
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("Writer"))
}

/// 설정 파일 경로 (`<config_dir>/Writer/config.toml`).
pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("config.toml"))
}

/// 새 라이브러리를 만들 때 제안하는 기본 경로 (`~/Documents/Writer`).
pub fn default_library_root() -> Option<PathBuf> {
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .map(|dir| dir.join("Writer"))
}

impl Config {
    /// 설정을 읽는다. 파일이 없거나 깨져 있으면 기본값을 돌려준다(앱 시작을 막지 않음).
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// 설정을 저장한다(상위 디렉터리 생성 포함).
    pub fn save(&self) -> io::Result<()> {
        let path = config_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "설정 디렉터리를 찾을 수 없습니다")
        })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        std::fs::write(path, text)
    }

    /// 라이브러리를 현재 루트로 설정하고 최근 목록을 갱신한다.
    pub fn set_library(&mut self, root: PathBuf) {
        self.recent_libraries.retain(|path| path != &root);
        self.recent_libraries.insert(0, root.clone());
        self.recent_libraries.truncate(RECENT_LIMIT);
        self.library_root = Some(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_library_updates_root_and_recents() {
        let mut config = Config::default();
        config.set_library(PathBuf::from("/a"));
        config.set_library(PathBuf::from("/b"));
        // 중복은 최신으로 승격
        config.set_library(PathBuf::from("/a"));

        assert_eq!(config.library_root, Some(PathBuf::from("/a")));
        assert_eq!(
            config.recent_libraries,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }
}
