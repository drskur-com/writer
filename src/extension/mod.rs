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

pub mod canvas;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use mlua::{Function, Lua, Table, Value, VmState};

pub use canvas::{Align, Color, Segment, Shape, Theme};

/// 라이브러리 루트 안의 확장 폴더.
pub const EXTENSIONS_DIR: &str = "Extensions";

/// 확장 파일 확장자.
const EXTENSION_FILE: &str = "lua";

/// 펜스 info string의 첫 단어.
const FENCE_TAG: &str = "ext";

/// 확장 하나가 쓸 수 있는 메모리.
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;

/// 파일을 읽어 표를 돌려받는 데 쓸 수 있는 시간.
const LOAD_TIMEOUT: Duration = Duration::from_secs(1);

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
    /// 캔버스 크기(논리 단위).
    pub size: (f32, f32),
    pub animated: bool,
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
        Self::load(name, &source)
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
        lua.sandbox(true).map_err(message)?;
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
        let size = canvas_size(&module)?;
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
        let recorder = Rc::new(RefCell::new(canvas::Recorder::new(size.0, size.1)));
        let g = canvas::create(&lua, recorder.clone()).map_err(message)?;
        Ok(Self {
            lua,
            module,
            g,
            recorder,
            deadline,
            size,
            animated,
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
        self.recorder
            .borrow_mut()
            .reset(self.size.0, self.size.1, theme.foreground);
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

fn canvas_size(module: &Table) -> Result<(f32, f32), String> {
    let size = match module.get::<Value>("size").map_err(message)? {
        Value::Nil => return Ok(DEFAULT_SIZE),
        Value::Table(size) => size,
        other => {
            return Err(format!(
                "size must be {{ width, height }}, got {}",
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

/// 읽은 확장(또는 읽지 못한 까닭).
pub type Loaded = Result<Extension, String>;

/// 열린 라이브러리의 확장 캐시. 파일 수정 시각이 바뀌면 다시 읽는다. UI 스레드에서만 쓴다.
pub struct Host {
    root: PathBuf,
    /// 이름 → (읽을 때의 파일 수정 시각, 읽은 확장).
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

    /// 확장 `name`. 처음이거나 파일이 바뀌었으면 읽는다.
    pub fn get(&self, name: &str) -> Rc<Loaded> {
        let modified = std::fs::metadata(path(&self.root, name))
            .and_then(|meta| meta.modified())
            .ok();
        if let Some((cached, loaded)) = self.cache.borrow().get(name)
            && *cached == modified
        {
            return loaded.clone();
        }
        let loaded = Rc::new(Extension::open(&self.root, name));
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
        assert_eq!(extension.size, (200.0, 100.0));
        assert!(extension.animated && extension.draws);
        assert_eq!(extension.commands, ["Upper", "Nothing"]);
        let args = [
            ("power".into(), "2".into()),
            ("label".into(), "명중".into()),
        ];
        let shapes = extension.draw(0.5, &args, &Theme::default()).unwrap();
        assert_eq!(shapes.len(), 3);
        assert!(matches!(&shapes[0], Shape::Fill { color, .. } if *color == [1.0, 0.0, 0.0, 1.0]));
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
    fn colon_calls_and_255_colors_work() {
        let extension = Extension::load(
            "x",
            "return { draw = function(g) g:setColor(255, 0, 0) g:circle('fill', 1, 1, 1) end }",
        )
        .unwrap();
        let shapes = extension.draw(0.0, &[], &Theme::default()).unwrap();
        assert!(matches!(&shapes[0], Shape::Fill { color, .. } if *color == [1.0, 0.0, 0.0, 1.0]));
    }
}
