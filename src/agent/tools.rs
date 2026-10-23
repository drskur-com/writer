//! 어시스턴트가 부르는 파일 도구. 라이브러리 폴더(작업 공간) 안만 읽고 쓴다.
//!
//! - 경로는 라이브러리 루트에서의 상대 경로다. 절대 경로, `..`, 루트 밖을 가리키는 심볼릭 링크는
//!   거부한다.
//! - 숨김 항목(`.writer/` 등 앱 상태)은 보이지도, 읽지도, 쓰지도 않는다. 그룹 메타(`.writer.toml`)만은
//!   읽고 `edit_file`로 고칠 수 있다(통째로 쓰지는 못한다).
//! - 새로 쓰기(`write_file`)는 글·설정·스크립트 파일만, 이미 있는 폴더 안에만 한다. 고치기
//!   (`edit_file`)는 있는 UTF-8 파일이면 된다([`super::edit`]). 둘 다 원자적으로 쓴다
//!   ([`crate::library::write_sheet`]).
//! - TOML 파일은 쓰기 전에 문법을, 그룹 메타는 앱이 읽는 모양까지 검사한다. 깨진 글은 쓰지 않는다.
//!
//! 라이브러리에 스킬(`Skills/`)이 있으면 스킬 본문을 읽는 `load_skill`도 준다([`super::skills`]).
//! `run_extension`은 확장(`Extensions/<이름>.lua`, ADR-0026)을 실행해 오류와 그린 결과를 알려 준다.
//! 모델이 쓴 확장을 스스로 확인하고 고치게 한다.
//!
//! 도구 설명과 결과(오류 포함)는 모델이 읽으므로 영어로 쓴다. 화면에 보이는 요약 줄(`summary`)만
//! 사용자 언어다.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use rig_core::completion::ToolDefinition;
use rig_core::message::ToolName;
use serde_json::{Map, Value, json};

use super::diff::{self, DiffLine};
use super::edit::{self, Edit};
use super::skills::{self, Skill};
use crate::library::GROUP_META_FILE;

/// 한 번에 읽을 수 있는 가장 큰 파일.
const MAX_READ_BYTES: u64 = 1024 * 1024;

/// `write_file`로 쓸 수 있는 파일 확장자: 글, 설정, 스킬 스크립트.
const WRITABLE_EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "toml", "json", "yaml", "yml", "csv", "py", "js", "mjs", "ts", "sh",
    "rb", "lua",
];

const LIST: &str = "list_files";
const READ: &str = "read_file";
const WRITE: &str = "write_file";
const EDIT: &str = "edit_file";
const CREATE_SHEET: &str = "create_sheet";
const LOAD_SKILL: &str = "load_skill";
const RUN_EXTENSION: &str = "run_extension";

/// 도구가 다룰 수 있는 폴더와 그 안의 스킬.
#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
    skills: Vec<Skill>,
    /// 확장 앞머리(ADR-0026). 시스템 프롬프트에 목록으로 넣는다.
    extensions: Vec<crate::extension::Meta>,
}

/// 파일을 바꾸는 도구의 계획. 사용자가 승인하면 [`Workspace::commit`]으로 쓴다.
#[derive(Debug)]
pub struct PendingChange {
    /// 바꿀 파일(라이브러리 루트 기준). 새 시트면 그룹 폴더.
    pub path: String,
    /// 지금 내용. 새 파일이면 `None`.
    pub before: Option<String>,
    pub after: String,
    target: Target,
    /// 쓴 뒤 모델에 돌려줄 글.
    done: String,
    /// 화면 요약 줄의 동사(`고침`, `씀`, `만듦`).
    verb: &'static str,
}

#[derive(Debug)]
enum Target {
    File(PathBuf),
    /// `dir`에 새 시트를 만든다. `group`은 루트 기준 그룹 경로.
    NewSheet {
        dir: PathBuf,
        group: String,
    },
}

impl PendingChange {
    /// 승인 카드에 보일 변경 내용.
    pub fn diff(&self) -> Vec<DiffLine> {
        match &self.before {
            Some(before) => diff::diff(before, &self.after),
            None => diff::added(&self.after),
        }
    }

    /// 새 파일(새 시트 포함)을 만드는지.
    pub fn is_new(&self) -> bool {
        self.before.is_none()
    }
}

/// 도구를 부른 계획: 곧바로 끝난 결과, 또는 승인을 기다리는 변경.
#[derive(Debug)]
pub enum Planned {
    Done(ToolOutput),
    Change(PendingChange),
}

/// 도구를 부른 결과.
#[derive(Debug)]
pub struct ToolOutput {
    /// 모델에 돌려줄 글(영어).
    pub text: String,
    /// 화면에 보일 한 줄(예: `읽음 · Projects/소설/1장.md`). 실패해도 까닭은 넣지 않는다(모델용 영어다).
    pub summary: String,
}

impl Workspace {
    /// `root`의 작업 공간. 스킬과 확장 목록은 지금 읽어 둔다(보낼 때마다 만들므로 늘 최신이다).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let skills = skills::load(&root);
        let extensions = crate::extension::catalog(&root);
        Self {
            root,
            skills,
            extensions,
        }
    }

    /// 라이브러리의 확장(이름순).
    pub fn extensions(&self) -> &[crate::extension::Meta] {
        &self.extensions
    }

    /// 쓸 수 있는 스킬(이름순).
    pub fn skills(&self) -> &[Skill] {
        &self.skills
    }

    /// 모델에 알릴 도구 정의.
    pub fn definitions(&self) -> Vec<Arc<ToolDefinition>> {
        let path = |description: &str| json!({ "type": "string", "description": description });
        let load_skill = (!self.skills.is_empty()).then(|| {
            (
                LOAD_SKILL,
                "Read a skill's instructions. Call it before answering when the request matches a skill's description, then follow the instructions.",
                json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "enum": self.skills.iter().map(|skill| skill.name.as_str()).collect::<Vec<_>>(),
                            "description": "Skill name.",
                        },
                    },
                    "required": ["name"],
                }),
            )
        });
        [
            (
                LIST,
                "List the entries of one folder in the library. Folder names end with `/`.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": path("Path relative to the library root. Empty for the root."),
                    },
                }),
            ),
            (
                READ,
                "Read one text file in the library and return its full content.",
                json!({
                    "type": "object",
                    "properties": { "path": path("Path relative to the library root.") },
                    "required": ["path"],
                }),
            ),
            (
                EDIT,
                "Change part of an existing file in the library (a sheet, a settings TOML, a skill, a script). \
                 For each change give old (a snippet of the current text, exactly as it is) and new (the replacement). \
                 old must occur exactly once in the file, so include enough surrounding text. Pass several changes \
                 in one call. If any change fails, nothing is changed.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": path("Path relative to the library root."),
                        "edits": {
                            "type": "array",
                            "minItems": 1,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "old": { "type": "string", "description": "The text to replace, exactly as in the file, including line breaks and spaces." },
                                    "new": { "type": "string", "description": "The replacement text. Empty to delete." },
                                    "all": { "type": "boolean", "description": "Replace every occurrence of old. Default false." },
                                },
                                "required": ["old", "new"],
                            },
                        },
                    },
                    "required": ["path", "edits"],
                }),
            ),
            (
                CREATE_SHEET,
                "Create a new sheet in a notes group or a project (manuscript) group. The app names the file \
                 (<ULID>.md) and, in manually ordered groups, appends it to the end of the sheet order. Returns \
                 the new sheet's path. A sheet's title is its first line, so content usually starts with a title \
                 line (# Title).",
                json!({
                    "type": "object",
                    "properties": {
                        "group": path("Group folder to put the sheet in, relative to the library root. For example Projects/Novel, Notes/Inbox."),
                        "content": { "type": "string", "description": "Sheet content (Markdown). Empty for a blank sheet." },
                    },
                    "required": ["group"],
                }),
            ),
            (
                RUN_EXTENSION,
                "Test an extension (Extensions/<name>.lua). Without command, calls draw(g, t, args) once and reports \
                 errors, the number of shapes, and the bounds of what was drawn compared with the canvas. With command, \
                 runs that command (1-based index or title) with the given selection and returns its result. Nothing \
                 in the library changes.",
                json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Extension name (the file name without .lua)." },
                        "t": { "type": "number", "description": "Time in seconds for draw. Default 0." },
                        "args": {
                            "type": "object",
                            "description": "Block arguments for draw, as in the ```ext block's key: value lines.",
                            "additionalProperties": { "type": ["string", "number", "boolean"] },
                        },
                        "command": { "type": ["integer", "string"], "description": "Command to run instead of draw: 1-based index or title." },
                        "selection": { "type": "string", "description": "Selected text passed to the command as ctx.selection." },
                    },
                    "required": ["name"],
                }),
            ),
            (
                WRITE,
                "Write one file that is not a sheet (settings, skills, scripts, and so on) in the library with the \
                 given content. An existing file is replaced entirely. The folder must already exist. Create new \
                 sheets with create_sheet, and change part of an existing file with edit_file.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": path("Path relative to the library root."),
                        "content": { "type": "string", "description": "The file's complete new content." },
                    },
                    "required": ["path", "content"],
                }),
            ),
        ]
        .into_iter()
        .chain(load_skill)
        .filter_map(|(name, description, parameters)| {
            let name = ToolName::new(name).ok()?;
            Some(Arc::new(ToolDefinition::new(name, description, parameters)))
        })
        .collect()
    }

    /// 도구를 승인 없이 부른다(변경도 바로 쓴다).
    #[cfg(test)]
    pub fn call(&self, name: &str, arguments: &Map<String, Value>) -> ToolOutput {
        match self.plan(name, arguments) {
            Planned::Done(output) => output,
            Planned::Change(change) => self.commit(change),
        }
    }

    /// 도구를 부른다. 읽기는 바로 끝내고, 파일을 바꾸는 도구는 쓰지 않고 계획만 돌려준다.
    /// 실패도 모델이 읽고 고칠 수 있게 글로 돌려준다.
    pub fn plan(&self, name: &str, arguments: &Map<String, Value>) -> Planned {
        if name == RUN_EXTENSION {
            let extension = arguments.get("name").and_then(Value::as_str).unwrap_or("");
            let path = format!("{}/{extension}.lua", crate::extension::EXTENSIONS_DIR);
            return Planned::Done(output(&path, "실행", self.run_extension(arguments)));
        }
        if name == LOAD_SKILL {
            return Planned::Done(
                self.load_skill(arguments.get("name").and_then(Value::as_str).unwrap_or("")),
            );
        }
        let path = arguments
            .get(if name == CREATE_SHEET {
                "group"
            } else {
                "path"
            })
            .and_then(Value::as_str)
            .unwrap_or("");
        let change = match name {
            CREATE_SHEET => {
                let content = arguments
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                self.plan_sheet(path, content)
            }
            WRITE => match arguments.get("content").and_then(Value::as_str) {
                Some(content) => self.plan_write(path, content),
                None => Err("content is missing.".into()),
            },
            EDIT => parse_edits(arguments).and_then(|edits| self.plan_edit(path, &edits)),
            LIST => return Planned::Done(output(path, "살펴봄", self.list(path))),
            READ => return Planned::Done(output(path, "읽음", self.read(path))),
            _ => Err(format!("Unknown tool: {name}")),
        };
        match change {
            Ok(change) => Planned::Change(change),
            Err(err) => Planned::Done(output(path, "", Err(err))),
        }
    }

    /// 승인한 변경을 쓴다. 기다리는 동안 파일이 바뀌었으면 쓰지 않는다.
    pub fn commit(&self, change: PendingChange) -> ToolOutput {
        let (path, verb) = (change.path.clone(), change.verb);
        output(&path, verb, self.write_change(change))
    }

    fn write_change(&self, change: PendingChange) -> Result<String, String> {
        match change.target {
            Target::File(file) => {
                let current = match std::fs::symlink_metadata(&file) {
                    Ok(_) => {
                        Some(crate::library::read_sheet(&file).map_err(|err| err.to_string())?)
                    }
                    Err(_) => None,
                };
                if current != change.before {
                    return Err(
                        "The file changed while waiting for approval. Read it again \
                                before retrying."
                            .into(),
                    );
                }
                crate::library::write_sheet(&file, &change.after).map_err(|err| err.to_string())?;
                Ok(change.done)
            }
            Target::NewSheet { dir, group } => self.create_sheet(&dir, &group, &change.after),
        }
    }

    fn load_skill(&self, name: &str) -> ToolOutput {
        match self.skills.iter().find(|skill| skill.name == name) {
            Some(skill) => ToolOutput {
                text: skills::instructions(skill),
                summary: format!("스킬 · {name}"),
            },
            None => ToolOutput {
                text: format!("Error: no such skill: {name}"),
                summary: format!("실패 · 스킬 {name}"),
            },
        }
    }

    /// 확장을 시험하고, 성공하면 앞머리에서 빠진 것을 덧붙인다(목록·Insert 대화상자가 앞머리를 쓴다).
    fn run_extension(&self, arguments: &Map<String, Value>) -> Result<String, String> {
        let report = self.try_extension(arguments)?;
        let name = arguments.get("name").and_then(Value::as_str).unwrap_or("");
        let draws = crate::extension::Extension::open(&self.root, name).is_ok_and(|ext| ext.draws);
        let meta = std::fs::read_to_string(crate::extension::path(&self.root, name))
            .ok()
            .and_then(|source| crate::extension::parse_meta(name, &source));
        let mut notes = Vec::new();
        match &meta {
            None => notes.push(
                "The file has no header. Start it with a --[[--- ... ---]] block holding description \
                 (in the user's language), args, and example; see the extension skill."
                    .to_string(),
            ),
            Some(meta) => {
                if meta.description.trim().is_empty() {
                    notes.push("The header has no description.".to_string());
                }
                let scale = crate::extension::SCALE_ARG;
                if meta.args.iter().any(|(arg, _)| arg == scale) {
                    notes.push(format!(
                        "`{scale}` is reserved: the app scales every block by it and does not pass it to the \
                         extension. Rename that argument."
                    ));
                }
                let described =
                    |key: &str| key == scale || meta.args.iter().any(|(arg, _)| arg == key);
                let undocumented: Vec<&str> = arguments
                    .get("args")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|args| args.keys())
                    .map(String::as_str)
                    .chain(meta.example.iter().map(|(key, _)| key.as_str()))
                    .filter(|key| !described(key))
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                if !undocumented.is_empty() {
                    notes.push(format!(
                        "Arguments not described under args in the header: {}.",
                        undocumented.join(", ")
                    ));
                }
                if draws && !meta.args.is_empty() && meta.example.is_empty() {
                    notes.push(
                        "The header has no example (the values used for the preview and when inserting the block)."
                            .to_string(),
                    );
                }
            }
        }
        Ok(if notes.is_empty() {
            report
        } else {
            format!("{report}\nNote: {}", notes.join(" "))
        })
    }

    fn try_extension(&self, arguments: &Map<String, Value>) -> Result<String, String> {
        use crate::extension::{CommandContext, Extension, Shape, Theme};
        let name = arguments.get("name").and_then(Value::as_str).unwrap_or("");
        let extension = Extension::open(&self.root, name)?;
        let commands = if extension.commands.is_empty() {
            "none".to_string()
        } else {
            extension
                .commands
                .iter()
                .enumerate()
                .map(|(ix, title)| format!("{}. {title}", ix + 1))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if let Some(command) = arguments.get("command") {
            let index = match command {
                Value::Number(n) => n.as_u64().and_then(|n| (n as usize).checked_sub(1)),
                Value::String(title) => extension.commands.iter().position(|t| t == title),
                _ => None,
            }
            .filter(|ix| *ix < extension.commands.len())
            .ok_or(format!("No such command. Commands: {commands}"))?;
            let ctx = CommandContext {
                selection: arguments
                    .get("selection")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                ..CommandContext::default()
            };
            return Ok(match extension.run_command(index, &ctx)? {
                Some(text) => format!("The command returned: {text:?}"),
                None => "The command returned nil (nothing is inserted).".into(),
            });
        }
        let t = arguments.get("t").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let args: Vec<(String, String)> = arguments
            .get("args")
            .and_then(Value::as_object)
            .map(|args| {
                args.iter()
                    .map(|(key, value)| {
                        let value = match value {
                            Value::String(text) => text.clone(),
                            other => other.to_string(),
                        };
                        (key.clone(), value)
                    })
                    .collect()
            })
            .unwrap_or_default();
        // 배율은 앱이 적용하고 확장에는 넘기지 않는다.
        let args = crate::extension::without_scale(&args);
        let (width, height) = extension.size_for(&args)?;
        let header = format!(
            "Canvas {width} x {height}, animated: {}, commands: {commands}.",
            extension.animated
        );
        if !extension.draws {
            return Ok(format!("{header} No draw function."));
        }
        let shapes = extension.draw(t, &args, &Theme::default())?;
        let (mut fills, mut strokes, mut texts) = (0, 0, 0);
        let mut bounds: Option<[f32; 4]> = None;
        let mut extend = |[x, y]: [f32; 2]| {
            let b = bounds.get_or_insert([x, y, x, y]);
            *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        };
        for shape in &shapes {
            match shape {
                Shape::Fill { path, .. } | Shape::Stroke { path, .. } => {
                    if matches!(shape, Shape::Fill { .. }) {
                        fills += 1;
                    } else {
                        strokes += 1;
                    }
                    for segment in path {
                        use crate::extension::Segment;
                        match *segment {
                            Segment::Move(p) | Segment::Line(p) => extend(p),
                            Segment::Cubic(a, b, p) => {
                                extend(a);
                                extend(b);
                                extend(p);
                            }
                            Segment::Close => {}
                        }
                    }
                }
                Shape::Text { origin, .. } => {
                    texts += 1;
                    extend(*origin);
                }
            }
        }
        let Some([x0, y0, x1, y1]) = bounds else {
            return Ok(format!("{header} At t = {t}: drew nothing."));
        };
        let outside = x0 < 0.0 || y0 < 0.0 || x1 > width || y1 > height;
        Ok(format!(
            "{header} At t = {t}: {} shapes ({fills} fills, {strokes} strokes, {texts} texts). \
             Drawing bounds x {x0:.1}..{x1:.1}, y {y0:.1}..{y1:.1} (text counts its top-left corner only).{}",
            shapes.len(),
            if outside {
                " Part of the drawing is outside the canvas and will be cut off."
            } else {
                ""
            }
        ))
    }

    fn list(&self, path: &str) -> Result<String, String> {
        let dir = self.resolve_existing(path)?;
        if !dir.is_dir() {
            return Err("Not a folder.".into());
        }
        let mut entries: Vec<String> = std::fs::read_dir(&dir)
            .map_err(|err| err.to_string())?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') && name != GROUP_META_FILE {
                    return None;
                }
                let is_dir = entry.file_type().ok()?.is_dir();
                Some(if is_dir { format!("{name}/") } else { name })
            })
            .collect();
        entries.sort();
        Ok(if entries.is_empty() {
            "(empty)".into()
        } else {
            entries.join("\n")
        })
    }

    fn read(&self, path: &str) -> Result<String, String> {
        let file = self.resolve_existing(path)?;
        let meta = std::fs::metadata(&file).map_err(|err| err.to_string())?;
        if !meta.is_file() {
            return Err("Not a file.".into());
        }
        if meta.len() > MAX_READ_BYTES {
            return Err(format!(
                "File too large ({} bytes, limit {MAX_READ_BYTES} bytes).",
                meta.len()
            ));
        }
        crate::library::read_sheet(&file).map_err(|err| match err.kind() {
            std::io::ErrorKind::InvalidData => "Not a text file (not UTF-8).".into(),
            _ => err.to_string(),
        })
    }

    fn plan_write(&self, path: &str, content: &str) -> Result<PendingChange, String> {
        let relative = relative_path(path)?;
        let name = relative.file_name().ok_or("The path has no file name.")?;
        let writable = Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| WRITABLE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));
        if name == GROUP_META_FILE {
            return Err(
                "Group settings cannot be written whole. Change them with edit_file.".into(),
            );
        }
        if !writable {
            return Err(format!(
                "This file type cannot be written (allowed: {}).",
                WRITABLE_EXTENSIONS.join(", ")
            ));
        }
        check_content(&relative, content)?;
        let parent = relative
            .parent()
            .map_or("", |parent| parent.to_str().unwrap_or(""));
        let dir = self.resolve_existing(parent)?;
        if !dir.is_dir() {
            return Err("The parent folder does not exist.".into());
        }
        let file = dir.join(name);
        // 새 시트는 이름 규칙(<ULID>.md)과 순서를 지키도록 create_sheet로만 만든다.
        if !file.exists() && is_markdown(name) && self.holds_sheets(parent) {
            return Err("Create new sheets with create_sheet.".into());
        }
        // 있는 파일이 심볼릭 링크면 루트 밖을 가리킬 수 있다.
        if file.is_symlink() {
            return Err("Cannot write to a symbolic link.".into());
        }
        if file.is_dir() {
            return Err("The path is a folder.".into());
        }
        let before = if file.exists() {
            Some(
                crate::library::read_sheet(&file)
                    .map_err(|_| "The existing file is not a text file (not UTF-8).".to_string())?,
            )
        } else {
            None
        };
        Ok(PendingChange {
            path: shown(path),
            before,
            after: content.to_string(),
            target: Target::File(file),
            done: format!("Wrote {} characters.", content.chars().count()),
            verb: "씀",
        })
    }

    fn plan_sheet(&self, group: &str, content: &str) -> Result<PendingChange, String> {
        let dir = self.resolve_existing(group)?;
        if !dir.is_dir() {
            return Err("Not a group folder.".into());
        }
        if !self.holds_sheets(group) {
            return Err(
                "Sheets cannot be created in this folder. Choose a sheet group under Notes/ or Projects/."
                    .into(),
            );
        }
        Ok(PendingChange {
            path: shown(group),
            before: None,
            after: content.to_string(),
            target: Target::NewSheet {
                dir,
                group: group.to_string(),
            },
            done: String::new(),
            verb: "만듦",
        })
    }

    fn create_sheet(&self, dir: &Path, group: &str, content: &str) -> Result<String, String> {
        let mut library = crate::library::scan(&self.root).map_err(|err| err.to_string())?;
        // 스캔한 경로는 정규화하지 않은 루트 기준이다.
        let group_dir = self.root.join(relative_path(group)?);
        let path = crate::library::create_sheet(dir).map_err(|err| err.to_string())?;
        if !content.is_empty()
            && let Err(err) = crate::library::write_sheet(&path, content)
        {
            std::fs::remove_file(&path).ok();
            return Err(err.to_string());
        }
        let manual = library.sort_of(&group_dir).0 == crate::library::SortMode::Manual;
        let mut note = String::new();
        if manual && let Some(group) = library.find_group_mut(&group_dir) {
            let sheet = group_dir.join(path.file_name().unwrap_or_default());
            if let Err(err) = crate::library::append_sheet(group, &sheet) {
                note = format!(" (could not save the sheet order: {err})");
            }
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let group = group.trim().trim_matches('/');
        Ok(format!("Created: {group}/{name}{note}"))
    }

    /// `path`(상대 경로)가 시트를 담는 폴더인지: `Notes/`·`Projects/` 아래이고 파일 그룹이 아니다.
    fn holds_sheets(&self, path: &str) -> bool {
        let Ok(relative) = relative_path(path) else {
            return false;
        };
        let in_section = relative.components().next().is_some_and(|part| {
            [
                crate::library::SectionKind::Notes,
                crate::library::SectionKind::Projects,
            ]
            .iter()
            .any(|kind| part.as_os_str() == kind.dir())
        });
        if !in_section {
            return false;
        }
        let kind = std::fs::read_to_string(self.root.join(&relative).join(GROUP_META_FILE))
            .ok()
            .and_then(|text| text.parse::<toml::Table>().ok())
            .and_then(|table| {
                table
                    .get("kind")
                    .and_then(|kind| kind.as_str().map(str::to_string))
            });
        kind.as_deref() != Some(crate::library::GroupKind::Files.key())
    }

    fn plan_edit(&self, path: &str, edits: &[Edit]) -> Result<PendingChange, String> {
        let file = self.resolve_existing(path)?;
        let text = self.read(path)?;
        let edited = edit::apply(&text, edits)?;
        check_content(&file, &edited.text)?;
        let places: Vec<String> = edited
            .places
            .iter()
            .map(|(line, count)| match count {
                1 => format!("line {line}"),
                _ => format!("{count} places from line {line}"),
            })
            .collect();
        Ok(PendingChange {
            path: shown(path),
            before: Some(text),
            after: edited.text,
            target: Target::File(file),
            done: format!("Changed: {}", places.join(", ")),
            verb: "고침",
        })
    }

    /// 있는 항목의 실제 경로. 루트 밖이거나 숨김 항목이면 거부한다.
    fn resolve_existing(&self, path: &str) -> Result<PathBuf, String> {
        let relative = relative_path(path)?;
        let root = self
            .root
            .canonicalize()
            .map_err(|err| format!("Cannot open the library folder: {err}"))?;
        let resolved = root
            .join(relative)
            .canonicalize()
            .map_err(|_| "No such path.".to_string())?;
        let inside = resolved
            .strip_prefix(&root)
            .map_err(|_| OUTSIDE.to_string())?;
        if is_hidden_path(inside) {
            return Err(HIDDEN.into());
        }
        Ok(resolved)
    }
}

/// 화면과 모델에 보일 경로. 비면 루트.
fn shown(path: &str) -> String {
    let path = path.trim().trim_matches('/');
    if path.is_empty() {
        "/".into()
    } else {
        path.into()
    }
}

/// 도구 결과. 실패해도 화면 요약 줄에는 까닭을 넣지 않는다(모델용 영어다).
fn output(path: &str, verb: &str, result: Result<String, String>) -> ToolOutput {
    let path = shown(path);
    match result {
        Ok(text) => ToolOutput {
            text,
            summary: format!("{verb} · {path}"),
        },
        Err(err) => ToolOutput {
            text: format!("Error: {err}"),
            summary: format!("실패 · {path}"),
        },
    }
}

fn is_markdown(name: &std::ffi::OsStr) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

/// `edit_file`의 `edits` 인자.
fn parse_edits(arguments: &Map<String, Value>) -> Result<Vec<Edit>, String> {
    let Some(items) = arguments.get("edits").and_then(Value::as_array) else {
        return Err("edits is missing (it must be an array).".into());
    };
    items
        .iter()
        .enumerate()
        .map(|(ix, item)| {
            let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
            Ok(Edit {
                old: text("old").ok_or(format!("Edit {} has no old.", ix + 1))?,
                new: text("new").ok_or(format!("Edit {} has no new.", ix + 1))?,
                all: item.get("all").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// 쓰기 전에 설정 파일이 깨지지 않았는지 본다.
fn check_content(path: &Path, content: &str) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if name == GROUP_META_FILE {
        return crate::library::validate_group_meta(content)
            .map_err(|err| format!("Not written. {err}"));
    }
    let is_toml = Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
    if is_toml {
        content
            .parse::<toml::Table>()
            .map_err(|err| format!("Not written. TOML syntax error: {err}"))?;
    }
    Ok(())
}

const OUTSIDE: &str = "Paths outside the library folder are not allowed.";
const HIDDEN: &str = "Hidden entries are not allowed.";

/// 모델이 준 경로를 검사한 상대 경로. 앞뒤의 `/`는 무시한다.
fn relative_path(path: &str) -> Result<PathBuf, String> {
    let path = Path::new(path.trim().trim_start_matches('/'));
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => relative.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(OUTSIDE.into());
            }
        }
    }
    if is_hidden_path(&relative) {
        return Err(HIDDEN.into());
    }
    Ok(relative)
}

/// 숨김 항목이 든 경로인지. 마지막의 그룹 메타(`.writer.toml`)는 숨김으로 치지 않는다.
fn is_hidden_path(path: &Path) -> bool {
    let parts: Vec<_> = path.components().collect();
    parts.iter().enumerate().any(|(ix, part)| {
        let name = part.as_os_str().to_string_lossy();
        name.starts_with('.') && !(ix + 1 == parts.len() && name == GROUP_META_FILE)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::test_util::temp_dir;

    fn args(pairs: &[(&str, &str)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), Value::from(*value)))
            .collect()
    }

    fn workspace() -> (Workspace, PathBuf) {
        let root = temp_dir("agent-tools");
        std::fs::create_dir_all(root.join(".writer")).unwrap();
        std::fs::write(root.join(".writer/library.toml"), "").unwrap();
        std::fs::create_dir_all(root.join("Projects/소설")).unwrap();
        std::fs::write(root.join("Projects/소설/1장.md"), "첫 장.").unwrap();
        std::fs::write(root.join("Projects/소설/.writer.toml"), "").unwrap();
        (Workspace::new(&root), root)
    }

    #[test]
    fn loads_skills_from_workspace() {
        let (_, root) = workspace();
        std::fs::create_dir_all(root.join("Skills/tone")).unwrap();
        std::fs::write(
            root.join("Skills/tone/SKILL.md"),
            "---\nname: tone\ndescription: 어조\n---\n존댓말로",
        )
        .unwrap();
        let ws = Workspace::new(&root);
        let names: Vec<_> = ws
            .definitions()
            .iter()
            .map(|def| def.name.to_string())
            .collect();
        assert_eq!(
            names,
            [
                LIST,
                READ,
                EDIT,
                CREATE_SHEET,
                RUN_EXTENSION,
                WRITE,
                LOAD_SKILL
            ]
        );
        let out = ws.call(LOAD_SKILL, &args(&[("name", "tone")]));
        assert!(out.text.starts_with("존댓말로"));
        assert_eq!(out.summary, "스킬 · tone");
        assert!(
            ws.call(LOAD_SKILL, &args(&[("name", "없음")]))
                .text
                .starts_with("Error:")
        );
        // 스킬이 없으면 load_skill을 주지 않는다.
        let (bare, _) = workspace();
        assert_eq!(bare.definitions().len(), 6);
    }

    #[test]
    fn lists_without_hidden_entries() {
        let (ws, _) = workspace();
        assert_eq!(ws.call(LIST, &args(&[])).text, "Projects/");
        // 그룹 메타만은 보인다.
        assert_eq!(
            ws.call(LIST, &args(&[("path", "Projects/소설")])).text,
            ".writer.toml\n1장.md"
        );
    }

    #[test]
    fn reads_and_writes_inside_root() {
        let (ws, root) = workspace();
        let read = ws.call(READ, &args(&[("path", "Projects/소설/1장.md")]));
        assert_eq!(read.text, "첫 장.");
        assert_eq!(read.summary, "읽음 · Projects/소설/1장.md");

        let wrote = ws.call(
            WRITE,
            &args(&[("path", "/Projects/소설/메모.txt"), ("content", "둘째 장.")]),
        );
        assert_eq!(wrote.summary, "씀 · Projects/소설/메모.txt");
        assert_eq!(
            std::fs::read_to_string(root.join("Projects/소설/메모.txt")).unwrap(),
            "둘째 장."
        );
        // 있는 시트는 통째로 쓸 수 있지만, 새 시트는 create_sheet로만 만든다.
        let out = ws.call(
            WRITE,
            &args(&[("path", "Projects/소설/1장.md"), ("content", "새 첫 장.")]),
        );
        assert_eq!(out.summary, "씀 · Projects/소설/1장.md");
        let out = ws.call(
            WRITE,
            &args(&[("path", "Projects/소설/2장.md"), ("content", "")]),
        );
        assert_eq!(out.text, "Error: Create new sheets with create_sheet.");
    }

    #[test]
    fn refuses_paths_outside_root_or_hidden() {
        let (ws, _) = workspace();
        for path in [
            "../outside.md",
            "Projects/../../outside.md",
            ".writer/library.toml",
        ] {
            let out = ws.call(READ, &args(&[("path", path)]));
            assert!(out.text.starts_with("Error:"), "{path}: {}", out.text);
        }
        let out = ws.call(
            WRITE,
            &args(&[("path", "Projects/소설/.writer.toml"), ("content", "")]),
        );
        assert!(
            out.text
                .starts_with("Error: Group settings cannot be written whole"),
            "{}",
            out.text
        );
        // 다른 숨김 항목은 고칠 수도 없다.
        let out = ws.call(EDIT, &edit_args(".writer/library.toml", "a", "b"));
        assert_eq!(out.text, format!("Error: {HIDDEN}"));
    }

    fn edit_args(path: &str, old: &str, new: &str) -> Map<String, Value> {
        json!({ "path": path, "edits": [{ "old": old, "new": new }] })
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn edits_sheet_in_place() {
        let (ws, root) = workspace();
        let out = ws.call(EDIT, &edit_args("Projects/소설/1장.md", "첫 장", "처음 장"));
        assert_eq!(out.text, "Changed: line 1");
        assert_eq!(out.summary, "고침 · Projects/소설/1장.md");
        assert_eq!(
            std::fs::read_to_string(root.join("Projects/소설/1장.md")).unwrap(),
            "처음 장."
        );
        let out = ws.call(EDIT, &edit_args("Projects/소설/1장.md", "없는 글", "x"));
        assert!(out.text.starts_with("Error:"), "{}", out.text);
        let out = ws.call(EDIT, &args(&[("path", "Projects/소설/1장.md")]));
        assert!(out.text.starts_with("Error: edits is missing"));
    }

    #[test]
    fn plans_without_writing_and_refuses_stale_commit() {
        let (ws, root) = workspace();
        let file = root.join("Projects/소설/1장.md");
        let Planned::Change(change) =
            ws.plan(EDIT, &edit_args("Projects/소설/1장.md", "첫 장", "처음 장"))
        else {
            panic!("edit_file must wait for approval");
        };
        assert_eq!(change.path, "Projects/소설/1장.md");
        assert!(!change.is_new());
        assert!(!change.diff().is_empty());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "첫 장.");
        // 기다리는 동안 사용자가 고쳤으면 쓰지 않는다.
        std::fs::write(&file, "사용자가 고친 장.").unwrap();
        let out = ws.commit(change);
        assert!(
            out.text.starts_with("Error: The file changed"),
            "{}",
            out.text
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "사용자가 고친 장.");
        // 읽기 도구는 승인 없이 끝난다.
        assert!(matches!(
            ws.plan(READ, &args(&[("path", "Projects/소설/1장.md")])),
            Planned::Done(_)
        ));
        let Planned::Change(change) = ws.plan(
            CREATE_SHEET,
            &args(&[("group", "Projects/소설"), ("content", "# 새")]),
        ) else {
            panic!("create_sheet must wait for approval");
        };
        assert!(change.is_new());
        assert_eq!(
            std::fs::read_dir(root.join("Projects/소설"))
                .unwrap()
                .count(),
            2
        );
    }

    #[test]
    fn creates_sheet_and_appends_to_manual_order() {
        let (ws, root) = workspace();
        std::fs::write(
            root.join("Projects/소설/.writer.toml"),
            "sheets = [\"1장.md\"]\n",
        )
        .unwrap();
        let out = ws.call(
            CREATE_SHEET,
            &args(&[("group", "Projects/소설"), ("content", "# 2장\n\n둘째.")]),
        );
        assert_eq!(out.summary, "만듦 · Projects/소설");
        let created = out.text.strip_prefix("Created: ").unwrap();
        let name = created.strip_prefix("Projects/소설/").unwrap();
        assert_eq!(name.len(), "01JB0000000000000000000000.md".len());
        assert_eq!(
            std::fs::read_to_string(root.join(created)).unwrap(),
            "# 2장\n\n둘째."
        );
        let meta = std::fs::read_to_string(root.join("Projects/소설/.writer.toml")).unwrap();
        let order: toml::Table = meta.parse().unwrap();
        let sheets: Vec<_> = order["sheets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|sheet| sheet.as_str().unwrap())
            .collect();
        assert_eq!(sheets, ["1장.md", name]);

        // 파일 그룹과 스킬 폴더에는 시트를 만들지 않는다.
        std::fs::create_dir_all(root.join("Projects/소설/Extras/Attachments")).unwrap();
        std::fs::write(
            root.join("Projects/소설/Extras/Attachments/.writer.toml"),
            "kind = \"files\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("Skills/tone")).unwrap();
        for group in ["Projects/소설/Extras/Attachments", "Skills/tone"] {
            let out = ws.call(CREATE_SHEET, &args(&[("group", group)]));
            assert!(
                out.text.starts_with("Error: Sheets cannot be created"),
                "{group}: {}",
                out.text
            );
        }
    }

    #[test]
    fn edits_group_meta_only_when_app_can_read_it() {
        let (ws, root) = workspace();
        let meta = root.join("Projects/소설/.writer.toml");
        std::fs::write(&meta, "title = \"소설\"\nsheets = [\"1장.md\"]\n").unwrap();
        let out = ws.call(
            EDIT,
            &edit_args("Projects/소설/.writer.toml", "\"소설\"", "\"장편\""),
        );
        assert_eq!(out.text, "Changed: line 1");
        // 시트 순서를 배열이 아닌 값으로 바꾸면 앱이 순서를 잃으므로 쓰지 않는다.
        let out = ws.call(
            EDIT,
            &edit_args("Projects/소설/.writer.toml", "[\"1장.md\"]", "\"1장.md\""),
        );
        assert!(out.text.starts_with("Error: Not written."), "{}", out.text);
        assert!(
            std::fs::read_to_string(&meta)
                .unwrap()
                .contains("[\"1장.md\"]")
        );
    }

    #[test]
    fn writes_skill_scripts_and_checks_toml() {
        let (ws, root) = workspace();
        std::fs::create_dir_all(root.join("Skills/count")).unwrap();
        let out = ws.call(
            WRITE,
            &args(&[("path", "Skills/count/count.py"), ("content", "print(1)\n")]),
        );
        assert_eq!(out.summary, "씀 · Skills/count/count.py");
        let out = ws.call(
            WRITE,
            &args(&[("path", "Skills/count/config.toml"), ("content", "a = ")]),
        );
        assert!(out.text.contains("TOML syntax error"), "{}", out.text);
        assert!(!root.join("Skills/count/config.toml").exists());
    }

    #[test]
    fn runs_extensions_for_the_model() {
        let (ws, root) = workspace();
        std::fs::create_dir_all(root.join("Extensions")).unwrap();
        std::fs::write(
            root.join("Extensions/dot.lua"),
            "--[[---\ndescription: 점\nargs:\n  r: 반지름\nexample:\n  r: 5\n---]]\n\
             return { size = { 100, 50 }, draw = function(g, t, args) g.circle('fill', 10 + t, 10, args.r) end,\n\
             commands = { { title = 'Shout', run = function(ctx) return ctx.selection .. '!' end } } }",
        )
        .unwrap();
        let run = |arguments: Value| ws.call(RUN_EXTENSION, arguments.as_object().unwrap());
        let out = run(json!({ "name": "dot", "args": { "r": 5 } }));
        assert_eq!(out.summary, "실행 · Extensions/dot.lua");
        assert!(out.text.contains("1 shapes (1 fills"), "{}", out.text);
        assert!(!out.text.contains("outside"), "{}", out.text);
        let out = run(json!({ "name": "dot", "t": 100, "args": { "r": 5 } }));
        assert!(out.text.contains("outside the canvas"), "{}", out.text);
        let out = run(json!({ "name": "dot", "command": "Shout", "selection": "hi" }));
        assert_eq!(out.text, "The command returned: \"hi!\"");
        let out = run(json!({ "name": "dot", "command": 2 }));
        assert!(
            out.text.starts_with("Error: No such command"),
            "{}",
            out.text
        );
        // 오류는 모델이 고칠 수 있게 그대로 돌려준다.
        let out = run(json!({ "name": "dot" }));
        assert!(out.text.starts_with("Error: "), "{}", out.text);
        let out = run(json!({ "name": "none" }));
        assert!(out.text.contains("No extension named"), "{}", out.text);
        assert!(
            !run(json!({ "name": "dot", "args": { "r": 5 } }))
                .text
                .contains("Note:")
        );
        // 앞머리에 없는 인자, 앞머리가 없는 확장은 알린다.
        let out = run(json!({ "name": "dot", "args": { "r": 5, "color": "red" } }));
        assert!(
            out.text
                .contains("not described under args in the header: color."),
            "{}",
            out.text
        );
        std::fs::write(
            root.join("Extensions/bare.lua"),
            "return { draw = function(g) g.circle('fill', 1, 1, 1) end }",
        )
        .unwrap();
        let out = run(json!({ "name": "bare" }));
        assert!(
            out.text.contains("Note: The file has no header."),
            "{}",
            out.text
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_out_of_root() {
        let (ws, root) = workspace();
        let outside = temp_dir("agent-tools-outside");
        std::fs::write(outside.join("secret.md"), "비밀").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("Projects/link")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.md"), root.join("Projects/s.md")).unwrap();
        let read = ws.call(READ, &args(&[("path", "Projects/link/secret.md")]));
        assert_eq!(read.text, format!("Error: {OUTSIDE}"));
        let write = ws.call(
            WRITE,
            &args(&[("path", "Projects/link/new.md"), ("content", "x")]),
        );
        assert_eq!(write.text, format!("Error: {OUTSIDE}"));
        let write = ws.call(WRITE, &args(&[("path", "Projects/s.md"), ("content", "x")]));
        assert!(write.text.starts_with("Error:"));
        assert_eq!(
            std::fs::read_to_string(outside.join("secret.md")).unwrap(),
            "비밀"
        );
    }

    #[test]
    fn writes_only_text_files_into_existing_folders() {
        let (ws, _) = workspace();
        let out = ws.call(
            WRITE,
            &args(&[("path", "Projects/cover.png"), ("content", "")]),
        );
        assert!(
            out.text.starts_with("Error: This file type"),
            "{}",
            out.text
        );
        let out = ws.call(
            WRITE,
            &args(&[("path", "Projects/없음/a.md"), ("content", "")]),
        );
        assert!(out.text.starts_with("Error:"));
        let out = ws.call(WRITE, &args(&[("path", "Projects/소설/1장.md")]));
        assert_eq!(out.text, "Error: content is missing.");
    }
}
