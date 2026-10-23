//! 확장(ADR-0026): 라이브러리 `Extensions/<이름>.lua`의 Lua(Luau) 스크립트.
//!
//! 확장 파일은 표 하나를 돌려주고, 앱은 그 안의 진입점만 쓴다.
//!
//! - `draw(g, t, args)`: 시트의 `` ```ext <이름> `` 펜스 자리에 그림을 그린다([`canvas`], LÖVE 모양 API).
//!   `t`는 초, `args`는 펜스 본문의 `key: value` 줄이다. `animated = true`면 보이는 동안 매 프레임 부른다.
//! - `commands = { { title = ..., run = function(ctx) ... end }, ... }`: `Extensions` 메뉴의 명령.
//!   돌려준 글로 선택 영역을 바꾸거나 커서 자리에 넣는다.
//!
//! 스크립트는 sandbox에서 돈다. 파일·프로세스·네트워크가 없고(Luau에는 `io`가 없고 `os`는 시간
//! 함수뿐이다), `require`는 뺐다. 메모리와 한 번 부를 때의 시간을 제한한다. `math.random`은 그릴 때마다
//! 같은 seed로 되돌려, 같은 `t`에서는 늘 같은 그림이 나온다(내보내기에서도 같아야 한다).
//!
//! 오류 글은 모델이 읽고 고치므로 영어로 쓴다.

mod boolean;
pub mod canvas;
pub(crate) mod path;
mod scene;
mod worker;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use mlua::{Function, Lua, Table, Value, VmState};

pub use canvas::{
    Align, Color, FillRule, LineCap, LineJoin, Paint, Segment, Shape, StrokeStyle, Theme,
};
pub use worker::{Info, Worker};

/// 선분 목록의 lyon 경로(화면 테셀레이션에 쓴다).
pub fn lyon_path(segments: &[Segment]) -> lyon::path::Path {
    path::LuaPath::to_lyon(segments)
}

/// 라이브러리 루트 안의 확장 폴더.
pub const EXTENSIONS_DIR: &str = "Extensions";

/// 확장 파일 확장자.
const EXTENSION_FILE: &str = "lua";

/// 펜스 info string의 첫 단어.
const FENCE_TAG: &str = "ext";

/// 확장 하나가 쓸 수 있는 메모리.
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;

/// 파일을 읽어 표를 돌려받는 데 쓸 수 있는 시간. 시트를 배치하려면 캔버스 크기가 필요해서 UI 스레드가
/// 이만큼 기다릴 수 있다. 확장의 최상위 코드는 표를 돌려주기만 하면 되므로 넉넉하다.
const LOAD_TIMEOUT: Duration = Duration::from_millis(100);

/// 반복 주기(`period`)의 최대 길이. 이보다 길면 프레임을 캐시하지 않는다.
const MAX_PERIOD: f32 = 10.0;

/// 한 프레임을 그리는 데 쓸 수 있는 시간.
const DRAW_TIMEOUT: Duration = Duration::from_millis(100);

/// 명령 하나에 쓸 수 있는 시간.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);

/// 이보다 큰 확장 파일은 읽지 않는다.
const MAX_SOURCE_BYTES: u64 = 512 * 1024;

/// `size`를 적지 않은 캔버스 크기.
const DEFAULT_SIZE: (f32, f32) = (400.0, 200.0);

/// 캔버스 한 변의 최대 크기.
const MAX_SIDE: f32 = 4096.0;

/// `math.random`의 seed. 그릴 때마다 되돌린다.
const RANDOM_SEED: i64 = 1;

/// 시트 안의 확장 블록 하나(`` ```ext <이름> `` 펜스).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// 펜스 여는 줄부터 닫는 줄까지(닫는 줄 포함).
    pub rows: Range<usize>,
    pub name: String,
    /// 본문의 `key: value` 줄. 순서대로.
    pub args: Vec<(String, String)>,
}

/// 앱이 읽는 블록 인자: 캔버스를 이 배율로 그린다(0.1~4, 기본 1). 확장에는 넘기지 않는다.
pub const SCALE_ARG: &str = "scale";

impl Block {
    /// 블록의 배율(`scale:`). 숫자가 아니면 1이다.
    pub fn scale(&self) -> f32 {
        scale_of(&self.args)
    }

    /// 확장에 넘길 인자(`scale` 뺌).
    pub fn extension_args(&self) -> Vec<(String, String)> {
        without_scale(&self.args)
    }
}

/// 인자의 배율(`scale:`). 없거나 숫자가 아니면 1이고, 0.1~4로 자른다.
pub fn scale_of(args: &[(String, String)]) -> f32 {
    args.iter()
        .rev()
        .find(|(key, _)| key == SCALE_ARG)
        .and_then(|(_, value)| value.parse::<f32>().ok())
        .filter(|scale| scale.is_finite())
        .map_or(1.0, |scale| scale.clamp(0.1, 4.0))
}

/// `scale`을 뺀 인자.
pub fn without_scale(args: &[(String, String)]) -> Vec<(String, String)> {
    args.iter()
        .filter(|(key, _)| key != SCALE_ARG)
        .cloned()
        .collect()
}

/// 본문에서 확장 블록을 찾는다. 닫지 않은 펜스는 블록이 아니다(쓰는 중이다).
/// 다른 코드 펜스 안의 `` ```ext ``는 글자일 뿐이다.
pub fn blocks(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut found = Vec::new();
    let mut row = 0;
    while row < lines.len() {
        let Some((fence, info)) = open_fence(lines[row]) else {
            row += 1;
            continue;
        };
        let close = (row + 1..lines.len()).find(|&ix| closes(lines[ix], fence));
        let Some(close) = close else {
            break;
        };
        let mut words = info.split_whitespace();
        if words.next() == Some(FENCE_TAG)
            && let Some(name) = words.next().filter(|name| valid_name(name))
        {
            found.push(Block {
                rows: row..close + 1,
                name: name.to_string(),
                args: parse_args(&lines[row + 1..close]),
            });
        }
        row = close + 1;
    }
    found
}

/// 펜스를 여는 줄이면 (펜스 글자와 길이, info string).
fn open_fence(line: &str) -> Option<((char, usize), &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let mark = trimmed.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let count = trimmed.chars().take_while(|c| *c == mark).count();
    if count < 3 {
        return None;
    }
    let info = &trimmed[count..];
    // 백틱 펜스의 info string에는 백틱이 없어야 한다(CommonMark).
    if mark == '`' && info.contains('`') {
        return None;
    }
    Some(((mark, count), info.trim()))
}

fn closes(line: &str, (mark, count): (char, usize)) -> bool {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return false;
    }
    let run = trimmed.chars().take_while(|c| *c == mark).count();
    run >= count && trimmed[run..].trim().is_empty()
}

/// 확장 이름: 글자·숫자·`-`·`_`만(파일 이름이 되므로 경로 문자는 안 된다).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
}

fn parse_args(lines: &[&str]) -> Vec<(String, String)> {
    lines
        .iter()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            let key = key.trim();
            (!key.is_empty()).then(|| (key.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// 확장 파일 경로.
pub fn path(root: &Path, name: &str) -> PathBuf {
    root.join(EXTENSIONS_DIR)
        .join(format!("{name}.{EXTENSION_FILE}"))
}

/// 라이브러리를 열 때 확장 폴더를 만들어 둔다(모델이 바로 파일을 쓸 수 있게).
pub fn ensure_dir(root: &Path) -> std::io::Result<()> {
    let dir = root.join(EXTENSIONS_DIR);
    if std::fs::symlink_metadata(&dir).is_ok() {
        return Ok(());
    }
    std::fs::create_dir(dir)
}

/// 확장 폴더의 확장 이름과 수정 시각(이름순). 바뀌었는지 비교할 때 쓴다.
pub fn scan(root: &Path) -> Vec<(String, Option<SystemTime>)> {
    let Ok(entries) = std::fs::read_dir(root.join(EXTENSIONS_DIR)) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some(EXTENSION_FILE) {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            let meta = std::fs::symlink_metadata(&path).ok()?;
            (valid_name(&name) && meta.is_file()).then(|| (name, meta.modified().ok()))
        })
        .collect();
    found.sort();
    found
}

/// 확장 폴더의 모든 명령: (확장 이름, 명령 번호, 제목). 읽지 못한 확장은 건너뛴다.
pub fn commands(root: &Path) -> Vec<(String, usize, String)> {
    scan(root)
        .into_iter()
        .filter_map(|(name, _)| Some((Extension::open(root, &name).ok()?, name)))
        .flat_map(|(extension, name)| {
            extension
                .commands
                .iter()
                .enumerate()
                .map(|(ix, title)| (name.clone(), ix, title.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// 확장 파일 맨 위의 앞머리(`--[[---` … `---]]` 블록 주석). Lua를 돌리지 않고 읽는다.
///
/// 목록·Insert 대화상자·시스템 프롬프트에 쓴다. 사람에게 보이는 글이라 설명은 사용자의 언어로 쓴다
/// (모델이 읽는 글을 영어로 쓰는 규칙의 예외).
///
/// ```lua
/// --[[---
/// name: archer
/// description: 궁수가 과녁에 화살을 쏜다.
/// args:
///   speed: 화살 속도. 기본값 1.
/// example:
///   speed: 1.5
/// ---]]
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// 확장 이름(파일 이름). 앞머리의 `name`이 다르면 파일 이름을 따른다(블록은 파일 이름으로 찾는다).
    pub name: String,
    pub description: String,
    /// 블록 인자와 그 설명. 순서대로.
    pub args: Vec<(String, String)>,
    /// 미리보기와 블록 넣기에 쓰는 인자 값.
    pub example: Vec<(String, String)>,
}

const META_OPEN: &str = "--[[---";
const META_CLOSE: &str = "---]]";

/// 앞머리를 읽는다. 파일이 앞머리로 시작하지 않으면(앞의 빈 줄은 괜찮다) `None`.
pub fn parse_meta(name: &str, source: &str) -> Option<Meta> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let rest = source.trim_start().strip_prefix(META_OPEN)?;
    let (body, _) = rest.split_once(META_CLOSE)?;
    let mut meta = Meta {
        name: name.to_string(),
        ..Meta::default()
    };
    let mut section: Option<&mut Vec<(String, String)>> = None;
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let indented = line.starts_with([' ', '\t']);
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if indented {
            if let Some(section) = section.as_deref_mut()
                && !key.is_empty()
            {
                section.push((key.to_string(), value.to_string()));
            }
            continue;
        }
        section = None;
        match key {
            "description" => meta.description = value.to_string(),
            "args" => section = Some(&mut meta.args),
            "example" => section = Some(&mut meta.example),
            _ => {}
        }
    }
    Some(meta)
}

/// 확장 폴더의 앞머리 목록(이름순). 앞머리가 없는 확장은 이름만 있다.
pub fn catalog(root: &Path) -> Vec<Meta> {
    scan(root)
        .into_iter()
        .map(|(name, _)| {
            std::fs::read_to_string(path(root, &name))
                .ok()
                .and_then(|source| parse_meta(&name, &source))
                .unwrap_or(Meta {
                    name,
                    ..Meta::default()
                })
        })
        .collect()
}

/// 시트에 넣을 블록 글(끝 줄바꿈 없음).
pub fn block_text(name: &str, args: &[(String, String)]) -> String {
    let mut text = format!("```{FENCE_TAG} {name}\n");
    for (key, value) in args {
        text.push_str(&format!("{key}: {value}\n"));
    }
    text.push_str("```");
    text
}

/// 명령에 넘기는 정보.
#[derive(Clone, Debug, Default)]
pub struct CommandContext {
    /// 선택한 글(없으면 빈 글).
    pub selection: String,
    /// 시트 본문 전체.
    pub text: String,
    /// 시트 경로(라이브러리 루트 기준).
    pub sheet: String,
}

/// 읽은 확장 하나. Lua 상태를 들고 있어 만든 스레드에서만 쓴다.
pub struct Extension {
    lua: Lua,
    module: Table,
    g: Table,
    recorder: Rc<RefCell<canvas::Recorder>>,
    deadline: Rc<Cell<Instant>>,
    /// 캔버스 크기(논리 단위). 블록 인자로 정하는 확장(`size = function(args)`)이면 `None`이고
    /// [`Self::size_for`]로 묻는다.
    pub size: Option<(f32, f32)>,
    size_fn: Option<Function>,
    pub animated: bool,
    /// 애니메이션이 되풀이되는 주기(초). 있으면 한 주기의 프레임을 캐시해 다시 그리지 않는다.
    pub period: Option<f32>,
    pub draws: bool,
    /// 명령 제목(순서대로).
    pub commands: Vec<String>,
}

impl std::fmt::Debug for Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extension")
            .field("size", &self.size)
            .field("animated", &self.animated)
            .field("draws", &self.draws)
            .field("commands", &self.commands)
            .finish_non_exhaustive()
    }
}

/// 확장 파일의 소스. 이름·파일 종류·크기를 검사한다.
pub fn read_source(root: &Path, name: &str) -> Result<String, String> {
    if !valid_name(name) {
        return Err(format!(
            "Invalid extension name \"{name}\" (use letters, digits, - and _)."
        ));
    }
    let file = path(root, name);
    let meta = std::fs::symlink_metadata(&file).map_err(|_| {
        format!("No extension named \"{name}\" ({EXTENSIONS_DIR}/{name}.{EXTENSION_FILE}).")
    })?;
    if !meta.is_file() {
        return Err(format!(
            "{EXTENSIONS_DIR}/{name}.{EXTENSION_FILE} is not a file."
        ));
    }
    if meta.len() > MAX_SOURCE_BYTES {
        return Err(format!(
            "The extension file is too large (limit {MAX_SOURCE_BYTES} bytes)."
        ));
    }
    let source = std::fs::read_to_string(&file).map_err(|err| err.to_string())?;
    Ok(source)
}

/// Lua 오류를 모델이 읽을 한 줄로.
fn message(err: mlua::Error) -> String {
    match err {
        mlua::Error::CallbackError { cause, traceback } => {
            let cause = message((*cause).clone());
            let line = traceback
                .lines()
                .map(str::trim)
                .find(|line| line.contains(".lua"))
                .map(|line| format!(" (at {line})"))
                .unwrap_or_default();
            format!("{cause}{line}")
        }
        mlua::Error::RuntimeError(text) | mlua::Error::SyntaxError { message: text, .. } => text,
        mlua::Error::MemoryError(_) => "out of memory (limit 64 MB)".into(),
        other => other.to_string(),
    }
}

impl Extension {
    /// `root`(라이브러리)의 확장 `name`을 읽는다.
    pub fn open(root: &Path, name: &str) -> Result<Self, String> {
        Self::load(name, &read_source(root, name)?)
    }

    /// 앱이 알아야 할 것(작업 스레드에서 UI 스레드로 넘긴다).
    pub fn info(&self) -> Info {
        Info {
            size: self.size,
            animated: self.animated,
            period: self.period,
            draws: self.draws,
            commands: self.commands.clone(),
        }
    }

    /// 소스에서 확장을 읽는다.
    pub fn load(name: &str, source: &str) -> Result<Self, String> {
        Self::try_load(name, source).map_err(|err| format!("{name}.lua: {err}"))
    }

    fn try_load(name: &str, source: &str) -> Result<Self, String> {
        let lua = Lua::new();
        lua.set_memory_limit(MEMORY_LIMIT).map_err(message)?;
        // 파일에서 다른 모듈을 읽지 못하게 한다.
        lua.globals()
            .raw_set("require", Value::Nil)
            .map_err(message)?;
        // 캔버스 `g`는 파일 맨 위에서도 쓸 수 있다(경로·그라디언트를 한 번 만들어 두고 draw에서 다시 쓴다).
        // 크기는 그릴 때마다 다시 정한다.
        let (width, height) = DEFAULT_SIZE;
        let recorder = Rc::new(RefCell::new(canvas::Recorder::new(width, height)));
        let g = canvas::create(&lua, recorder.clone()).map_err(message)?;
        lua.sandbox(true).map_err(message)?;
        // sandbox가 그 전 전역 표를 읽기 전용으로 얼리므로(`g.theme`을 그릴 때마다 바꾼다) 그 뒤에 넣는다.
        lua.globals().raw_set("g", g.clone()).map_err(message)?;
        let deadline = Rc::new(Cell::new(Instant::now() + LOAD_TIMEOUT));
        let check = deadline.clone();
        lua.set_interrupt(move |_| {
            if Instant::now() > check.get() {
                Err(mlua::Error::runtime(
                    "took too long (an endless loop?); each call has a time limit",
                ))
            } else {
                Ok(VmState::Continue)
            }
        });
        let value: Value = lua
            .load(source)
            .set_name(format!("@{name}.lua"))
            .eval()
            .map_err(message)?;
        let Value::Table(module) = value else {
            return Err(format!(
                "the file must return a table (return {{ draw = ..., commands = ... }}), got {}",
                value.type_name()
            ));
        };
        let draws = matches!(module.get::<Value>("draw"), Ok(Value::Function(_)));
        let commands = command_titles(&module)?;
        if !draws && commands.is_empty() {
            return Err("the returned table has neither draw nor commands".into());
        }
        let (size, size_fn) = match module.get::<Value>("size").map_err(message)? {
            Value::Function(size_fn) => (None, Some(size_fn)),
            value => (Some(canvas_size(value)?), None),
        };
        let animated = match module.get::<Value>("animated").map_err(message)? {
            Value::Nil => false,
            Value::Boolean(animated) => animated,
            other => {
                return Err(format!(
                    "animated must be true or false, got {}",
                    other.type_name()
                ));
            }
        };
        let period = match module.get::<Value>("period").map_err(message)? {
            Value::Nil => None,
            Value::Integer(n) => Some(n as f32),
            Value::Number(n) => Some(n as f32),
            other => {
                return Err(format!(
                    "period must be a number of seconds, got {}",
                    other.type_name()
                ));
            }
        };
        if let Some(period) = period
            && !(period > 0.0 && period <= MAX_PERIOD)
        {
            return Err(format!("period must be between 0 and {MAX_PERIOD} seconds"));
        }
        Ok(Self {
            lua,
            module,
            g,
            recorder,
            deadline,
            size,
            size_fn,
            animated,
            period,
            draws,
            commands,
        })
    }

    /// `t`초의 그림. `args`는 블록 본문이다.
    pub fn draw(
        &self,
        t: f32,
        args: &[(String, String)],
        theme: &Theme,
    ) -> Result<Vec<Shape>, String> {
        if !self.draws {
            return Err("this extension has no draw function".into());
        }
        let draw: Function = self.module.get("draw").map_err(message)?;
        let (width, height) = self.size_for(args)?;
        self.recorder.borrow_mut().reset(
            width,
            height,
            theme.foreground,
            if self.animated {
                canvas::ANIMATED_SHAPE_LIMIT
            } else {
                canvas::STILL_SHAPE_LIMIT
            },
        );
        let run = || -> mlua::Result<()> {
            self.g
                .raw_set("theme", canvas::theme_table(&self.lua, theme)?)?;
            let math: Table = self.lua.globals().get("math")?;
            math.get::<Function>("randomseed")?
                .call::<()>(RANDOM_SEED)?;
            self.deadline.set(Instant::now() + DRAW_TIMEOUT);
            draw.call::<()>((self.g.clone(), t, self.args_table(args)?))
        };
        run().map_err(message)?;
        Ok(std::mem::take(&mut self.recorder.borrow_mut().shapes))
    }

    /// 블록 인자 `args`일 때의 캔버스 크기.
    pub fn size_for(&self, args: &[(String, String)]) -> Result<(f32, f32), String> {
        let Some(size_fn) = &self.size_fn else {
            return Ok(self.size.unwrap_or(DEFAULT_SIZE));
        };
        let call = || -> mlua::Result<Value> {
            self.deadline.set(Instant::now() + DRAW_TIMEOUT);
            size_fn.call(self.args_table(args)?)
        };
        canvas_size(call().map_err(message)?).map_err(|err| format!("size(args): {err}"))
    }

    /// 명령 `index`를 부른다. 넣을 글을 돌려준다(`nil`이면 `None`).
    pub fn run_command(
        &self,
        index: usize,
        ctx: &CommandContext,
    ) -> Result<Option<String>, String> {
        let commands: Table = self.module.get("commands").map_err(message)?;
        let command: Table = commands.get(index + 1).map_err(message)?;
        let run: Function = command.get("run").map_err(message)?;
        let call = || -> mlua::Result<Value> {
            let table = self.lua.create_table()?;
            table.raw_set("selection", ctx.selection.as_str())?;
            table.raw_set("text", ctx.text.as_str())?;
            table.raw_set("sheet", ctx.sheet.as_str())?;
            self.deadline.set(Instant::now() + COMMAND_TIMEOUT);
            run.call(table)
        };
        match call().map_err(message)? {
            Value::Nil => Ok(None),
            Value::String(text) => Ok(Some(text.to_string_lossy())),
            Value::Integer(n) => Ok(Some(n.to_string())),
            Value::Number(n) => Ok(Some(n.to_string())),
            other => Err(format!(
                "run must return a string or nil, got {}",
                other.type_name()
            )),
        }
    }

    /// 블록 본문을 Lua 표로. 숫자·`true`·`false`로 읽히면 그 값으로 넘긴다.
    fn args_table(&self, args: &[(String, String)]) -> mlua::Result<Table> {
        let table = self.lua.create_table()?;
        for (key, value) in args {
            let value = if let Ok(n) = value.parse::<f64>() {
                Value::Number(n)
            } else if value == "true" || value == "false" {
                Value::Boolean(value == "true")
            } else {
                Value::String(self.lua.create_string(value)?)
            };
            table.raw_set(key.as_str(), value)?;
        }
        Ok(table)
    }
}

fn command_titles(module: &Table) -> Result<Vec<String>, String> {
    let commands = match module.get::<Value>("commands").map_err(message)? {
        Value::Nil => return Ok(Vec::new()),
        Value::Table(commands) => commands,
        other => {
            return Err(format!(
                "commands must be a list of {{ title = ..., run = function(ctx) ... end }}, got {}",
                other.type_name()
            ));
        }
    };
    commands
        .sequence_values::<Value>()
        .enumerate()
        .map(|(ix, command)| {
            let Ok(Value::Table(command)) = command else {
                return Err(format!("commands[{}] must be a table", ix + 1));
            };
            let title: Option<String> = command.get("title").ok();
            let title = title
                .filter(|title| !title.trim().is_empty())
                .ok_or_else(|| format!("commands[{}] needs a title string", ix + 1))?;
            if !matches!(command.get::<Value>("run"), Ok(Value::Function(_))) {
                return Err(format!("commands[{}] needs a run function", ix + 1));
            }
            Ok(title)
        })
        .collect()
}

/// `size` 값(또는 `size(args)`가 돌려준 값): `{ width, height }`. 없으면 기본 크기.
fn canvas_size(value: Value) -> Result<(f32, f32), String> {
    let size = match value {
        Value::Nil => return Ok(DEFAULT_SIZE),
        Value::Table(size) => size,
        other => {
            return Err(format!(
                "size must be {{ width, height }} or function(args) returning it, got {}",
                other.type_name()
            ));
        }
    };
    let side = |ix: usize| -> Result<f32, String> {
        let value: f32 = size
            .get::<Option<f32>>(ix)
            .ok()
            .flatten()
            .ok_or("size must be { width, height } (two numbers)")?;
        if !(1.0..=MAX_SIDE).contains(&value) {
            return Err(format!("size values must be between 1 and {MAX_SIDE}"));
        }
        Ok(value)
    };
    Ok((side(1)?, side(2)?))
}

/// 작업 스레드에 띄운 확장(또는 띄우지 못한 까닭).
pub type Loaded = Result<Worker, String>;

/// 열린 시트(또는 대화상자)의 확장 캐시: 확장마다 작업 스레드 하나. 파일 수정 시각이 바뀌면 새로 띄우고
/// 옛 스레드는 요청 통로가 끊겨 끝난다. UI 스레드에서만 쓴다.
pub struct Host {
    root: PathBuf,
    /// 이름 → (띄울 때의 파일 수정 시각, 띄운 확장).
    cache: RefCell<HashMap<String, Cached>>,
}

type Cached = (Option<SystemTime>, Rc<Loaded>);

impl Host {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            cache: RefCell::default(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 확장 `name`. 처음이거나 파일이 바뀌었으면 띄운다(읽기가 끝날 때까지 기다린다).
    pub fn get(&self, name: &str) -> Rc<Loaded> {
        let modified = std::fs::metadata(path(&self.root, name))
            .and_then(|meta| meta.modified())
            .ok();
        if let Some((cached, loaded)) = self.cache.borrow().get(name)
            && *cached == modified
        {
            return loaded.clone();
        }
        let loaded =
            Rc::new(read_source(&self.root, name).and_then(|source| Worker::spawn(name, source)));
        self.cache
            .borrow_mut()
            .insert(name.to_string(), (modified, loaded.clone()));
        loaded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ext_fences_with_args() {
        let text = "앞\n```ext archer\npower: 3\ntarget: 오크\n```\n```rust\n```ext inner\n```\n뒤";
        assert_eq!(
            blocks(text),
            [Block {
                rows: 1..5,
                name: "archer".into(),
                args: vec![
                    ("power".into(), "3".into()),
                    ("target".into(), "오크".into())
                ],
            }]
        );
        // 닫지 않은 펜스, 이름이 없거나 경로 문자가 든 이름은 블록이 아니다.
        assert!(blocks("```ext archer\npower: 3").is_empty());
        assert!(blocks("```ext\n```").is_empty());
        assert!(blocks("```ext ../x\n```").is_empty());
        // 물결 펜스와 긴 펜스도 된다.
        assert_eq!(blocks("~~~~ ext 별\n~~~~").len(), 1);
    }

    #[test]
    fn reads_meta_without_running_lua() {
        let source = "\n--[[---\nname: other\ndescription: 궁수가 쏜다.\nargs:\n  speed: 화살 속도. 기본값 1.\n  label: 글\nexample:\n  speed: 1.5\nunknown: x\n---]]\nreturn {}";
        assert_eq!(
            parse_meta("archer", source),
            Some(Meta {
                name: "archer".into(),
                description: "궁수가 쏜다.".into(),
                args: vec![
                    ("speed".into(), "화살 속도. 기본값 1.".into()),
                    ("label".into(), "글".into())
                ],
                example: vec![("speed".into(), "1.5".into())],
            })
        );
        // 앞머리가 맨 위에 없거나 닫히지 않으면 없는 것이다.
        assert_eq!(
            parse_meta("x", "return {}\n--[[---\ndescription: d\n---]]"),
            None
        );
        assert_eq!(parse_meta("x", "--[[---\ndescription: d\n"), None);
        // 앞머리는 Lua 주석이라 실행에 영향이 없다.
        assert!(Extension::load("archer", source).is_err_and(|err| err.contains("neither")));
        // 넣은 블록은 다시 블록으로 읽힌다.
        let text = block_text("archer", &[("speed".into(), "1.5".into())]);
        assert_eq!(
            blocks(&text)[0].args,
            [("speed".to_string(), "1.5".to_string())]
        );
    }

    const ARCHER: &str = r#"
return {
  size = { 200, 100 },
  animated = true,
  draw = function(g, t, args)
    g.setColor(1, 0, 0)
    g.circle("fill", 10, 10, 5)
    g.push()
    g.translate(100, 0)
    g.rectangle("line", 0, 0, 10 * args.power, 10)
    g.pop()
    g.print(args.label, 0, 50)
  end,
  commands = {
    { title = "Upper", run = function(ctx) return string.upper(ctx.selection) end },
    { title = "Nothing", run = function(ctx) end },
  },
}
"#;

    #[test]
    fn draws_shapes_in_canvas_space() {
        let extension = Extension::load("archer", ARCHER).unwrap();
        assert_eq!(extension.size, Some((200.0, 100.0)));
        assert!(extension.animated && extension.draws);
        assert_eq!(extension.commands, ["Upper", "Nothing"]);
        let args = [
            ("power".into(), "2".into()),
            ("label".into(), "명중".into()),
        ];
        let shapes = extension.draw(0.5, &args, &Theme::default()).unwrap();
        assert_eq!(shapes.len(), 3);
        assert!(
            matches!(&shapes[0], Shape::Fill { paint, .. } if paint.color() == [1.0, 0.0, 0.0, 1.0])
        );
        let Shape::Stroke { path, .. } = &shapes[1] else {
            panic!("{:?}", shapes[1]);
        };
        assert_eq!(path[0], Segment::Move([100.0, 0.0]));
        assert_eq!(path[1], Segment::Line([120.0, 0.0]));
        assert!(matches!(&shapes[2], Shape::Text { text, .. } if text == "명중"));
        // 다시 그리면 변환과 모양이 처음부터다.
        assert_eq!(
            extension.draw(0.5, &args, &Theme::default()).unwrap(),
            shapes
        );
    }

    const VECTOR: &str = r##"
local leaf = g.newPath():moveTo(0, 0):curveTo(10, -10, 30, -10, 40, 0):quadTo(20, 10, 0, 0):close()
local open = g.newPath("M0 0 C 0 20 40 20 40 0 Z")
local shut = g.newPath("M0 0 C 0 0 40 0 40 0 Z")
local road = g.newPath("M0 50 L 100 50")
return {
  size = {100, 100},
  draw = function(g, t)
    g.setColor(g.newLinearGradient(0, 0, 100, 0, {{0, "#ff0000"}, {0.5, "#00ff00"}, {1, "#0000ff"}}))
    g.push()
    g.translate(10, 10)
    g.fill(leaf)
    g.pop()
    g.setColor(g.newRadialGradient(50, 50, 30, {"#ffffff", {1, 1, 1, 0}}))
    g.circle("fill", 50, 50, 30)
    g.setColor(1, 0, 0)
    g.setLineJoin("round"); g.setLineCap("round"); g.setLineWidth(4)
    g.setDash({10, 10}, 5)
    g.stroke(road)
    g.setDash()
    g.setScissor(0, 0, 50, 100)
    g.fill(g.lerpPath(open, shut, g.ease.inOutSine(0.5)))
    g.setScissor()
    local x, y, angle = road:pointAt(0.25)
    g.stroke(road:trim(0, 0.5):translate(0, 10))
    g.print(string.format("%d %d %.2f %d", x, y, angle, road:length()), 0, 0)
  end,
}
"##;

    #[test]
    fn draws_vector_paths_and_gradients() {
        let extension = Extension::load("vector", VECTOR).unwrap();
        let shapes = extension.draw(0.0, &[], &Theme::default()).unwrap();
        let Shape::Fill { path, paint, .. } = &shapes[0] else {
            panic!("{:?}", shapes[0]);
        };
        // 변환을 따라 옮겨지고, 2차 곡선은 3차로 바뀐다.
        assert_eq!(path[0], Segment::Move([10.0, 10.0]));
        assert!(matches!(path[2], Segment::Cubic(..)));
        assert!(matches!(paint, Paint::Linear { stops, .. } if stops.len() == 3));
        assert!(
            matches!(&shapes[1], Shape::Fill { paint: Paint::Radial { radius, .. }, .. } if *radius == 30.0)
        );
        // 점선: 길이 100을 10/10 무늬로 5만큼 밀면 조각 여섯.
        let Shape::Stroke { path, style, .. } = &shapes[2] else {
            panic!("{:?}", shapes[2]);
        };
        assert_eq!(style.join, LineJoin::Round);
        assert_eq!(style.cap, LineCap::Round);
        assert_eq!(
            path.iter()
                .filter(|s| matches!(s, Segment::Move(_)))
                .count(),
            6
        );
        assert_eq!(shapes[3], Shape::Clip(Some([0.0, 0.0, 50.0, 100.0])));
        // 모핑: 가운데는 높이 10의 곡선.
        let Shape::Fill { path, .. } = &shapes[4] else {
            panic!("{:?}", shapes[4]);
        };
        assert_eq!(
            path[1],
            Segment::Cubic([0.0, 10.0], [40.0, 10.0], [40.0, 0.0])
        );
        assert_eq!(shapes[5], Shape::Clip(None));
        let Shape::Stroke { path, .. } = &shapes[6] else {
            panic!("{:?}", shapes[6]);
        };
        assert_eq!(path.last(), Some(&Segment::Line([50.0, 60.0])));
        assert!(matches!(&shapes[7], Shape::Text { text, .. } if text == "25 50 0.00 100"));
    }

    #[test]
    fn clips_and_combines_paths() {
        let source = r##"
local A = g.newPath():rect(0, 0, 10, 10)
local B = g.newPath():rect(5, 0, 10, 10)
return {
  size = { 100, 100 },
  draw = function(g)
    g.translate(20, 0)
    g.setClip(g.newPath():circle(5, 5, 5))
    g.rectangle("fill", 0, 0, 10, 10)
    g.setClip()
    g.origin()
    g.fill(A:union(B))
    g.fill(A:subtract(B))
    g.fill(g.newPath("M0 0 L 50 0"):outline(4, "round", "round"))
    g.fill(A:offset(2))
    local x, y, w, h = A:intersect(B):bounds()
    g.print(string.format("%g %g %g %g", x, y, w, h), 0, 0)
  end,
}
"##;
        let extension = Extension::load("clip", source).unwrap();
        let shapes = extension.draw(0.0, &[], &Theme::default()).unwrap();
        // 자르는 경로도 변환을 따른다.
        let Shape::ClipPath(Some((path, FillRule::NonZero))) = &shapes[0] else {
            panic!("{:?}", shapes[0]);
        };
        assert_eq!(path[0], Segment::Move([30.0, 5.0]));
        assert_eq!(shapes[2], Shape::ClipPath(None));
        let points = |ix: usize| match &shapes[ix] {
            Shape::Fill { path, .. } => path.len(),
            other => panic!("{other:?}"),
        };
        assert!(points(3) >= 5 && points(4) >= 5);
        assert!(points(5) > 10);
        assert!(
            matches!(&shapes[7], Shape::Text { text, .. } if text == "5 0 5 10"),
            "{:?}",
            shapes[7]
        );
    }

    #[test]
    fn vector_errors_name_what_is_wrong() {
        let draw = |body: &str| {
            let source = format!("return {{ size = {{10, 10}}, draw = function(g) {body} end }}");
            Extension::load("bad", &source)
                .unwrap()
                .draw(0.0, &[], &Theme::default())
                .unwrap_err()
        };
        assert!(draw("g.fill(g.newPath('M0 0 X'))").contains("unknown path command"));
        assert!(draw("g.newPath():bend()").contains("path:bend does not exist"));
        assert!(
            draw("g.lerpPath(g.newPath('M0 0 L1 1'), g.newPath('M0 0 C1 1 1 1 1 1'), 0.5)")
                .contains("same structure")
        );
        assert!(draw("g.setLineJoin('sharp')").contains("line join"));
        assert!(draw("g.newLinearGradient(0, 0, 1, 1, {})").contains("at least one color stop"));
        assert!(draw("g.fill(3)").contains("must be a path"));
        assert!(draw("g.newPath('M0 0 L1 0'):outline(0)").contains("more than 0"));
        assert!(draw("g.setClip('M0 0 L1 0 L1 1 Z', 'odd')").contains("fill rule"));
    }

    #[test]
    fn runs_commands() {
        let extension = Extension::load("archer", ARCHER).unwrap();
        let ctx = CommandContext {
            selection: "abc".into(),
            ..CommandContext::default()
        };
        assert_eq!(extension.run_command(0, &ctx), Ok(Some("ABC".into())));
        assert_eq!(extension.run_command(1, &ctx), Ok(None));
    }

    #[test]
    fn reports_errors_the_model_can_fix() {
        let err = Extension::load("x", "return 1").unwrap_err();
        assert!(err.contains("must return a table"), "{err}");
        let err = Extension::load("x", "return {}").unwrap_err();
        assert!(err.contains("neither draw nor commands"), "{err}");
        let err = Extension::load("x", "return { draw = function(g) g.fooBar() end }")
            .unwrap()
            .draw(0.0, &[], &Theme::default())
            .unwrap_err();
        assert!(err.contains("g.fooBar does not exist"), "{err}");
        let err = Extension::load("x", "return { draw = function(g) g.circle('fill', 1) end }")
            .unwrap()
            .draw(0.0, &[], &Theme::default())
            .unwrap_err();
        assert!(
            err.contains("g.circle: argument #3 must be a number"),
            "{err}"
        );
    }

    #[test]
    fn sandbox_blocks_files_and_endless_loops() {
        for source in [
            "return { draw = function(g) io.open('x') end }",
            "return { draw = function(g) os.execute('ls') end }",
            "return { draw = function(g) require('x') end }",
        ] {
            let extension = Extension::load("x", source).unwrap();
            assert!(
                extension.draw(0.0, &[], &Theme::default()).is_err(),
                "{source}"
            );
        }
        let extension =
            Extension::load("x", "return { draw = function(g) while true do end end }").unwrap();
        let err = extension.draw(0.0, &[], &Theme::default()).unwrap_err();
        assert!(err.contains("took too long"), "{err}");
        // 시간 함수는 쓸 수 있다.
        let extension = Extension::load(
            "x",
            "return { commands = { { title = 'd', run = function() return os.date('%Y') end } } }",
        )
        .unwrap();
        assert_eq!(
            extension
                .run_command(0, &CommandContext::default())
                .unwrap()
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn random_is_the_same_every_frame() {
        let extension = Extension::load(
            "x",
            "return { draw = function(g) g.circle('fill', math.random() * 100, 0, 1) end }",
        )
        .unwrap();
        let theme = Theme::default();
        assert_eq!(
            extension.draw(0.0, &[], &theme).unwrap(),
            extension.draw(1.0, &[], &theme).unwrap()
        );
    }

    #[test]
    fn scale_is_read_by_the_app() {
        let block = &blocks("```ext dot\nr: 3\nscale: 2\n```")[0];
        assert_eq!(block.scale(), 2.0);
        assert_eq!(block.extension_args(), [("r".to_string(), "3".to_string())]);
        assert_eq!(scale_of(&[("scale".into(), "x".into())]), 1.0);
        assert_eq!(scale_of(&[("scale".into(), "100".into())]), 4.0);
        assert_eq!(scale_of(&[]), 1.0);
    }

    #[test]
    fn size_can_follow_block_args() {
        let extension = Extension::load(
            "x",
            "return { size = function(args) return { 100 * (args.scale or 1), 50 } end, \
             draw = function(g) g.rectangle('fill', 0, 0, g.getWidth(), g.getHeight()) end }",
        )
        .unwrap();
        assert_eq!(extension.size, None);
        let args = [("scale".to_string(), "2".to_string())];
        assert_eq!(extension.size_for(&args), Ok((200.0, 50.0)));
        assert_eq!(extension.size_for(&[]), Ok((100.0, 50.0)));
        // 캔버스(g.getWidth)도 그 크기다.
        let shapes = extension.draw(0.0, &args, &Theme::default()).unwrap();
        let Shape::Fill { path, .. } = &shapes[0] else {
            panic!()
        };
        assert!(path.contains(&Segment::Line([200.0, 0.0])));
        let bad = Extension::load(
            "x",
            "return { size = function() return { 0, 1 } end, draw = function() end }",
        )
        .unwrap();
        assert!(bad.size_for(&[]).unwrap_err().contains("between 1 and"));
    }

    /// `extension` 스킬의 예제는 모델이 그대로 베끼므로 모두 실제로 돌아야 한다.
    #[test]
    fn skill_examples_run() {
        let doc = include_str!("../../resources/skills/extension/references/examples.md");
        let sources: Vec<&str> = doc
            .split("```lua\n")
            .skip(1)
            .filter_map(|rest| rest.split("\n```").next())
            .collect();
        assert_eq!(sources.len(), 7);
        let theme = Theme::default();
        let args = [
            ("values".to_string(), "12, 30, 21, 8".to_string()),
            ("labels".to_string(), "봄, 여름, 가을, 겨울".to_string()),
            ("label".to_string(), "명중!".to_string()),
            ("sway".to_string(), "1".to_string()),
            ("from".to_string(), "서울".to_string()),
        ];
        for source in sources {
            let extension = Extension::load("example", source).unwrap();
            // 앞머리가 있고, 그림을 그리면 example 값이 모두 args에 설명되어 있다.
            let meta = parse_meta("example", source).unwrap();
            assert!(!meta.description.is_empty());
            for (key, _) in &meta.example {
                assert!(meta.args.iter().any(|(arg, _)| arg == key), "{key}");
            }
            if extension.draws {
                assert!(!meta.example.is_empty());
                extension.draw(0.0, &meta.example, &theme).unwrap();
            }
            if extension.draws {
                for t in [0.0, 0.7, 1.9, 2.5] {
                    for args in [&args[..], &[]] {
                        let shapes = extension.draw(t, args, &theme).unwrap();
                        assert!(!shapes.is_empty());
                        let (w, h) = extension.size_for(args).unwrap();
                        let inside = |[x, y]: [f32; 2]| {
                            (-1.0..=w + 1.0).contains(&x) && (-1.0..=h + 1.0).contains(&y)
                        };
                        for shape in &shapes {
                            let points: Vec<[f32; 2]> = match shape {
                                Shape::Fill { path, .. } | Shape::Stroke { path, .. } => path
                                    .iter()
                                    .filter_map(|segment| match *segment {
                                        Segment::Move(p) | Segment::Line(p) => Some(p),
                                        Segment::Cubic(_, _, p) => Some(p),
                                        Segment::Close => None,
                                    })
                                    .collect(),
                                Shape::Text { origin, .. } => vec![*origin],
                                Shape::Clip(_) | Shape::ClipPath(_) => Vec::new(),
                            };
                            assert!(points.into_iter().all(inside), "t = {t}: {shape:?}");
                        }
                    }
                }
            }
            for ix in 0..extension.commands.len() {
                for selection in ["", "가나다\nabc"] {
                    let ctx = CommandContext {
                        selection: selection.into(),
                        ..CommandContext::default()
                    };
                    extension.run_command(ix, &ctx).unwrap();
                }
            }
        }
        // 글자 단위 명령은 한글을 깨뜨리지 않는다.
        let tools = Extension::load(
            "text-tools",
            doc.split("```lua\n")
                .nth(4)
                .unwrap()
                .split("\n```")
                .next()
                .unwrap(),
        )
        .unwrap();
        let ctx = CommandContext {
            selection: "가나다".into(),
            ..CommandContext::default()
        };
        assert_eq!(tools.run_command(2, &ctx), Ok(Some("다나가".into())));
        assert_eq!(tools.run_command(1, &ctx), Ok(Some("「가나다」".into())));
    }

    #[test]
    fn draws_a_3d_scene() {
        let extension = Extension::load(
            "x",
            r#"return { size = { 200, 100 }, draw = function(g)
                local s = g.newScene()
                s:camera(0, 0, 5, 0, 0, 0)
                s:setColor(1, 0, 0)
                s:box("fill", 1)
                s:line(-1, 0, 1, 1, 0, 1)
                s:draw()
                local x, y = s:project(0, 0, 0)
                g.print(string.format("%d,%d", x, y), 0, 0)
            end }"#,
        )
        .unwrap();
        let shapes = extension.draw(0.0, &[], &Theme::default()).unwrap();
        // 정면에서 본 상자는 앞면 하나만 보이고, 선은 상자 앞(먼저 그린 면 위)에 온다.
        assert_eq!(shapes.len(), 3, "{shapes:?}");
        assert!(
            matches!(&shapes[0], Shape::Fill { paint, .. } if paint.color()[0] > 0.3 && paint.color()[1] == 0.0),
            "{shapes:?}"
        );
        assert!(matches!(&shapes[1], Shape::Stroke { .. }));
        assert!(matches!(&shapes[2], Shape::Text { text, .. } if text == "100,50"));

        let err = Extension::load(
            "x",
            "return { draw = function(g) g.newScene():teapot('fill') end }",
        )
        .unwrap()
        .draw(0.0, &[], &Theme::default())
        .unwrap_err();
        assert!(err.contains("scene:teapot does not exist"), "{err}");
        let err = Extension::load(
            "x",
            "return { draw = function(g) g.newScene():sphere('fill', 'big') end }",
        )
        .unwrap()
        .draw(0.0, &[], &Theme::default())
        .unwrap_err();
        assert!(err.contains("scene:sphere: argument #2"), "{err}");
    }

    #[test]
    fn colon_calls_and_255_colors_work() {
        let extension = Extension::load(
            "x",
            "return { draw = function(g) g:setColor(255, 0, 0) g:circle('fill', 1, 1, 1) end }",
        )
        .unwrap();
        let shapes = extension.draw(0.0, &[], &Theme::default()).unwrap();
        assert!(
            matches!(&shapes[0], Shape::Fill { paint, .. } if paint.color() == [1.0, 0.0, 0.0, 1.0])
        );
    }
}
