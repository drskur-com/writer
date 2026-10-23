//! 여러 파일로 된 확장(ADR-0039): `Extensions/<이름>/init.lua`가 진입 파일이고, 같은 폴더의 다른 `.lua`는
//! `require("모듈")`로 부르는 모듈이다. 파일 하나(`Extensions/<이름>.lua`)는 모듈 없는 확장이다.
//!
//! - 모듈 이름은 폴더 안 경로를 점으로 잇는다: `ui/hud.lua` → `ui.hud`, `ui/init.lua` → `ui`.
//!   경로 조각은 확장 이름과 같은 규칙(글자·숫자·`-`·`_`)이다.
//! - 앱이 열 때 폴더의 `.lua`를 모두 읽어 둔다. Lua 쪽에는 여전히 파일 접근이 없고, `require`는 그 안에서만
//!   찾는다. 모듈은 한 번만 실행하고 돌려준 값을 다시 쓴다. 서로 부르면 오류다.
//! - 숨김 항목·심볼릭 링크·`.lua`가 아닌 파일은 무시한다. 파일 수·전체 크기·폴더 깊이에 상한이 있다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use mlua::{Lua, Table, Value};

use super::{EXTENSION_FILE, EXTENSIONS_DIR, MAX_SOURCE_BYTES, valid_name};

/// 폴더 확장의 진입 파일.
pub const ENTRY_FILE: &str = "init.lua";

/// 폴더 확장의 모듈 파일 수 상한(진입 파일 빼고).
pub const MAX_MODULES: usize = 64;

/// 폴더 확장 전체 소스의 크기 상한.
pub const MAX_PACKAGE_BYTES: u64 = 4 * MAX_SOURCE_BYTES;

/// 폴더 안 깊이 상한(`a/b/c.lua`가 3).
pub const MAX_DEPTH: usize = 4;

/// 확장 하나의 소스: 진입 파일과 모듈(이름 → (폴더 안 경로, 소스)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Package {
    pub main: String,
    pub modules: BTreeMap<String, (String, String)>,
    /// `g.newImage`가 그림을 찾는 곳. 라이브러리에서 읽은 확장만 있다.
    pub images: Option<super::image::ImageRoots>,
}

impl From<String> for Package {
    fn from(main: String) -> Self {
        Self {
            main,
            modules: BTreeMap::new(),
            images: None,
        }
    }
}

impl From<&str> for Package {
    fn from(main: &str) -> Self {
        main.to_string().into()
    }
}

impl Package {
    /// 폴더 안 경로(`ui/hud.lua`)와 소스들로 만든다. `init.lua`가 진입 파일이다.
    pub fn from_files<'a>(
        files: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, String> {
        let mut main = None;
        let mut modules = BTreeMap::new();
        for (path, source) in files {
            if path == ENTRY_FILE {
                main = Some(source.to_string());
                continue;
            }
            let name = module_name(path).ok_or_else(|| {
                format!(
                    "{path}: module paths use letters, digits, - and _ in each part and end in .{EXTENSION_FILE}"
                )
            })?;
            if let Some((other, _)) =
                modules.insert(name.clone(), (path.to_string(), source.to_string()))
            {
                return Err(format!("{other} and {path} are both module \"{name}\""));
            }
        }
        if modules.len() > MAX_MODULES {
            return Err(format!("too many modules (limit {MAX_MODULES})"));
        }
        let main = main.ok_or_else(|| format!("the folder has no {ENTRY_FILE}"))?;
        Ok(Self {
            main,
            modules,
            images: None,
        })
    }

    /// 모듈이 있는지(폴더 확장인지).
    pub fn is_folder(&self) -> bool {
        !self.modules.is_empty()
    }
}

/// 폴더 안 경로의 모듈 이름. 모듈이 될 수 없는 경로면 `None`.
pub fn module_name(path: &str) -> Option<String> {
    let stem = path.strip_suffix(&format!(".{EXTENSION_FILE}"))?;
    let mut parts: Vec<&str> = stem.split('/').collect();
    if parts.len() > MAX_DEPTH || !parts.iter().all(|part| valid_name(part)) {
        return None;
    }
    if parts.len() > 1 && parts.last() == Some(&"init") {
        parts.pop();
    }
    (parts != ["init"]).then(|| parts.join("."))
}

/// 확장 폴더(있든 없든).
pub fn folder(root: &Path, name: &str) -> PathBuf {
    root.join(EXTENSIONS_DIR).join(name)
}

/// 확장의 파일 하나(`<이름>.lua`) 경로(있든 없든).
pub fn single_file(root: &Path, name: &str) -> PathBuf {
    root.join(EXTENSIONS_DIR)
        .join(format!("{name}.{EXTENSION_FILE}"))
}

/// 확장이 폴더인지(진입 파일이 있는 진짜 폴더).
pub fn is_folder(root: &Path, name: &str) -> bool {
    let dir = folder(root, name);
    std::fs::symlink_metadata(&dir).is_ok_and(|meta| meta.is_dir())
        && std::fs::symlink_metadata(dir.join(ENTRY_FILE)).is_ok_and(|meta| meta.is_file())
}

/// 폴더의 `.lua` 파일들: (폴더 안 경로, 실제 경로), 경로순. 숨김·심볼릭 링크는 건너뛴다.
fn lua_files(dir: &Path) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let relative = format!("{prefix}{name}");
            if kind.is_dir() && depth < MAX_DEPTH {
                walk(&entry.path(), &format!("{relative}/"), depth + 1, out);
            } else if kind.is_file() && name.ends_with(&format!(".{EXTENSION_FILE}")) {
                out.push((relative, entry.path()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, "", 1, &mut out);
    out.sort();
    out
}

/// 폴더 확장의 소스를 읽는다(크기·개수 검사).
pub fn read_folder(dir: &Path) -> Result<Package, String> {
    let mut total = 0;
    let mut files = Vec::new();
    for (relative, file) in lua_files(dir) {
        let size = std::fs::metadata(&file)
            .map_err(|err| err.to_string())?
            .len();
        if size > MAX_SOURCE_BYTES {
            return Err(format!(
                "{relative} is too large (limit {MAX_SOURCE_BYTES} bytes)."
            ));
        }
        total += size;
        if total > MAX_PACKAGE_BYTES {
            return Err(format!(
                "The extension folder is too large (limit {MAX_PACKAGE_BYTES} bytes in all)."
            ));
        }
        let source = std::fs::read_to_string(&file).map_err(|err| format!("{relative}: {err}"))?;
        files.push((relative, source));
    }
    Package::from_files(
        files
            .iter()
            .map(|(path, source)| (path.as_str(), source.as_str())),
    )
}

/// 확장이 마지막으로 바뀐 때: 파일이면 그 파일, 폴더면 안의 파일·폴더 중 가장 늦은 것(지우거나 더해도 바뀐다).
pub fn modified(root: &Path, name: &str) -> Option<SystemTime> {
    if !is_folder(root, name) {
        return std::fs::metadata(single_file(root, name))
            .and_then(|meta| meta.modified())
            .ok();
    }
    fn latest(dir: &Path, depth: usize) -> Option<SystemTime> {
        let mut newest = std::fs::metadata(dir).and_then(|meta| meta.modified()).ok();
        for entry in std::fs::read_dir(dir).ok()?.filter_map(Result::ok) {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let time = if meta.is_dir() && depth < MAX_DEPTH {
                latest(&entry.path(), depth + 1)
            } else {
                meta.modified().ok()
            };
            newest = newest.max(time);
        }
        newest
    }
    latest(&folder(root, name), 1)
}

/// Lua 상태에 `require`를 둔다(sandbox 전에 부른다). 모듈은 `package`에서만 찾고, 처음 부를 때 한 번
/// 실행해 돌려준 값(없으면 `true`)을 다시 쓴다.
pub fn install_require(lua: &Lua, extension: &str, package: &Package) -> mlua::Result<()> {
    let modules = Rc::new(package.modules.clone());
    let loaded: Table = lua.create_table()?;
    let extension = extension.to_string();
    let loading = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let require = lua.create_function(move |lua, name: String| {
        let cached: Value = loaded.raw_get(name.as_str())?;
        if !cached.is_nil() {
            return Ok(cached);
        }
        if loading.borrow().contains(&name) {
            let chain = loading.borrow().join(" -> ");
            return Err(mlua::Error::runtime(format!(
                "circular require: {chain} -> {name}"
            )));
        }
        let Some((path, source)) = modules.get(&name) else {
            let known: Vec<&str> = modules.keys().map(String::as_str).collect();
            return Err(mlua::Error::runtime(if known.is_empty() {
                format!(
                    "no module \"{name}\": this extension is one file; put modules in {EXTENSIONS_DIR}/{extension}/ next to {ENTRY_FILE}"
                )
            } else {
                format!("no module \"{name}\" (modules: {})", known.join(", "))
            }));
        };
        loading.borrow_mut().push(name.clone());
        let value = lua
            .load(source.as_str())
            .set_name(format!("@{extension}/{path}"))
            .eval::<Value>();
        loading.borrow_mut().pop();
        let value = match value? {
            Value::Nil => Value::Boolean(true),
            value => value,
        };
        loaded.raw_set(name.as_str(), value.clone())?;
        Ok(value)
    })?;
    lua.globals().raw_set("require", require)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_names_follow_the_folder() {
        assert_eq!(module_name("enemies.lua").as_deref(), Some("enemies"));
        assert_eq!(module_name("ui/hud.lua").as_deref(), Some("ui.hud"));
        assert_eq!(module_name("ui/init.lua").as_deref(), Some("ui"));
        assert_eq!(module_name("init.lua"), None);
        assert_eq!(module_name("ui/my file.lua"), None);
        assert_eq!(module_name("notes.txt"), None);
        assert_eq!(module_name("a/b/c/d/e.lua"), None);
    }

    #[test]
    fn packages_need_an_entry_and_unique_modules() {
        let package =
            Package::from_files([("init.lua", "return {}"), ("ui/hud.lua", "return 1")]).unwrap();
        assert_eq!(package.main, "return {}");
        assert_eq!(package.modules["ui.hud"].0, "ui/hud.lua");
        assert!(
            Package::from_files([("a.lua", "")])
                .unwrap_err()
                .contains("init.lua")
        );
        assert!(
            Package::from_files([("init.lua", ""), ("ui.lua", ""), ("ui/init.lua", "")])
                .unwrap_err()
                .contains("both module \"ui\"")
        );
    }
}
