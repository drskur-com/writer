# Folder extensions: several files with `require`

A large extension (a game with levels, a chart kit with shared helpers, an illustration with many parts) can be a folder instead of one file:

```
Extensions/space-game/
  init.lua          -- the entry: header and the returned table, like a one-file extension
  enemies.lua       -- require("enemies")
  ui/hud.lua        -- require("ui.hud")
  ui/init.lua       -- require("ui")
```

- `init.lua` is the extension: it starts with the header (`--[[---` … `---]]`) and returns the table with `draw`, `play`, `commands`, and so on. Blocks and inline calls use the folder name (`` ```ext space-game ``).
- Every other `.lua` file in the folder is a module. Its name is its path inside the folder with `.` for `/` and without `.lua`; `ui/init.lua` is `ui`. Each part uses letters, digits, `-`, and `_`, at most 4 levels deep. Other files are ignored.
- `require("name")` runs the module once, the first time, and returns what it returned (later calls get the same value). A module that returns nothing gives `true`. Modules that require each other in a circle are an error. `require` sees only this extension's modules: not other extensions, not files.
- Modules share the sandbox, limits (64 MB, 0.1 s to load everything), and the canvas global `g` with `init.lua`. Limits: 64 modules, 512 KB per file, 2 MB in all.
- A one-file extension cannot `require`. Prefer one file until it passes a few hundred lines or has clearly separate parts.

## Module shape

Return a table of functions and constants; keep state in `init.lua` (or in a table it passes in), so it is clear what `load` resets:

```lua
-- Extensions/space-game/enemies.lua
local M = {}

function M.spawn(list, x)
  table.insert(list, { x = x, y = -10, speed = 40 + math.random() * 40 })
end

function M.update(list, dt)
  for _, e in ipairs(list) do e.y = e.y + e.speed * dt end
end

function M.draw(g, list)
  g.setColor(g.theme.accent)
  for _, e in ipairs(list) do g.circle("fill", e.x, e.y, 6) end
end

return M
```

```lua
-- Extensions/space-game/init.lua
--[[---
name: space-game
description: 떨어지는 적을 피하는 게임. ← → 로 움직인다.
---]]
local enemies = require("enemies")
local hud = require("ui.hud")
local list, x, score
return {
  export = false,
  draw = function(g) hud.title(g, "Space game — ▶ to play") end,
  play = {
    load = function() list, x, score = {}, 180, 0 end,
    update = function(dt) enemies.update(list, dt) end,
    draw = function(g) enemies.draw(g, list); hud.score(g, score) end,
  },
}
```

## Writing and testing

- Write each file with `write_file` at its path (`Extensions/space-game/init.lua`, `Extensions/space-game/enemies.lua`); the folders appear when the user approves. All files of the extension are one draft, `run_extension` (by the folder name) runs the folder with your drafts, and the user approves the whole folder at once.
- To turn a one-file extension into a folder, write `Extensions/<name>/init.lua` and its modules: on approval the folder replaces `Extensions/<name>.lua`, which moves to the trash, so move everything it needs into the folder.
- Errors name the file (`space-game/enemies.lua:12: …`); fix that file with `edit_file`.
