//! 프로젝트 컴파일(내보내기): 원고 전체를 하나의 완성된 전자책이나 문서로 묶는다.
//!
//! ePub(전자책), PDF(인쇄/문서), DOCX(워드), HTML 등 다양한 형식으로 내보낼 수 있으며,
//! 책 정보(`.writer.toml`의 `[book]`)와 책 스타일(`[styles.*]`)을 그대로 반영한다.

pub mod epub;

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

use crate::library::{self, BookMeta, Styles};

/// 컴파일 중 생길 수 있는 오류.
#[derive(Debug)]
pub enum CompileError {
    Io(io::Error),
    Zip(zip::result::ZipError),
    ProjectNotFound(PathBuf),
    InvalidProject(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "입출력 오류: {err}"),
            Self::Zip(err) => write!(f, "ZIP 압축 오류: {err}"),
            Self::ProjectNotFound(path) => write!(f, "프로젝트를 찾을 수 없습니다: {}", path.display()),
            Self::InvalidProject(msg) => write!(f, "잘못된 프로젝트: {msg}"),
        }
    }
}

impl std::error::Error for CompileError {}

impl From<io::Error> for CompileError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<zip::result::ZipError> for CompileError {
    fn from(err: zip::result::ZipError) -> Self {
        Self::Zip(err)
    }
}

/// 컴파일할 프로젝트 원고 묶음.
pub struct CompileProject {
    pub name: String,
    pub dir: PathBuf,
    pub book: BookMeta,
    pub styles: Styles,
    pub sheets: Vec<CompileSheet>,
}

/// 컴파일에 들어가는 시트 하나.
pub struct CompileSheet {
    pub path: PathBuf,
    /// 앞 시트에 이어지는지(ADR-0012). 이어지면 새 장으로 분리하지 않고 이전 파일에 연결된다.
    pub glued: bool,
    /// 시트 원문 텍스트.
    pub content: String,
}

/// 프로젝트 폴더에서 컴파일에 필요한 모든 정보를 읽는다.
pub fn load_compile_project(project_dir: &Path) -> Result<CompileProject, CompileError> {
    let root = project_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| CompileError::InvalidProject("상위 라이브러리 폴더를 찾을 수 없습니다".into()))?;

    let library = library::scan(root)?;
    let project = library
        .find_project(project_dir)
        .ok_or_else(|| CompileError::ProjectNotFound(project_dir.to_path_buf()))?;

    let book = library::read_book(project_dir);
    let styles = library::read_styles(project_dir);
    let manuscript = library::manuscript_sheets(project);

    let mut sheets = Vec::with_capacity(manuscript.len());
    for ms in manuscript {
        let content = library::read_sheet(&ms.path).unwrap_or_default();
        sheets.push(CompileSheet {
            path: ms.path,
            glued: ms.glued,
            content,
        });
    }

    Ok(CompileProject {
        name: project.name.clone(),
        dir: project_dir.to_path_buf(),
        book,
        styles,
        sheets,
    })
}

/// 프로젝트 폴더의 원고를 ePub 형식으로 컴파일하여 지정한 파일로 저장한다.
pub fn compile_epub_to_file(project_dir: &Path, output_path: &Path) -> Result<(), CompileError> {
    let project = load_compile_project(project_dir)?;
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(output_path)?;
    let mut writer = BufWriter::new(file);
    epub::build_epub(&project, &mut writer)?;
    Ok(())
}
