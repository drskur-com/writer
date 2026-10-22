---
name: extension
description: Write or fix a Writer extension — a Lua script in Extensions/ that draws a picture or animation inside a sheet (a ```ext block) or adds a command to the Extensions menu (for example inserting today's date or transforming the selected text). Use when the user wants something drawn or animated in the editor, a custom text command, or when an extension shows an error.
---

# Writer extensions

Answer in the user's language. Code, comments, and file names in English unless the user asks otherwise.

## Files and blocks

- One extension = one file: `Extensions/<name>.lua` (`<name>`: letters, digits, `-`, `_`). Create it with `write_file`, change it with `edit_file`. The app reloads it when the file changes.
- A sheet shows a drawing with a fenced block. The body lines are `key: value` arguments (numbers, `true`/`false`, or text):

  ````
  ```ext archer
  power: 3
  label: Hit!
  ```
  ````

  Add or change the block in the sheet with `edit_file` (sheets are Markdown; keep a blank line before and after the block).
- Commands appear in the app's **Extensions** menu. The user runs them on the open sheet.

## The script

The file returns one table. Use only the fields you need:

```lua
return {
  size = { 400, 160 },   -- canvas width, height in canvas units (default 400 x 200). Shrunk to fit the page width.
  animated = true,       -- call draw every frame while visible (default false: draw once with t = 0)
  draw = function(g, t, args)
    -- g: the canvas (below). t: seconds since the block appeared. args: the block's key: value lines.
  end,
  commands = {
    { title = "Insert today's date", run = function(ctx)
        return os.date("%Y-%m-%d")   -- the returned string replaces the selection (or is inserted at the cursor); nil does nothing
      end },
  },
}
```

`ctx` (commands): `ctx.selection` (selected text, may be empty), `ctx.text` (whole sheet), `ctx.sheet` (sheet path).

## Canvas `g` — the LÖVE `love.graphics` API

Coordinates are canvas units, origin top-left, y down, angles in radians. Colors are 0–1 (`r, g, b[, a]`), a table `{r, g, b, a}`, or `"#rrggbb"`. The default color is the theme's text color.

| Function | Notes |
|---|---|
| `g.setColor(r, g, b, a)`, `g.getColor()` | |
| `g.setLineWidth(w)`, `g.getLineWidth()` | |
| `g.setFont(size)` / `g.setFont(g.newFont(size))` | A font is just its size (default 12) |
| `g.getWidth()`, `g.getHeight()`, `g.getDimensions()` | Canvas size |
| `g.clear(r, g, b, a)` | Fills the whole canvas |
| `g.rectangle(mode, x, y, w, h[, rx, ry])` | `mode`: `"fill"` or `"line"` |
| `g.circle(mode, x, y, r)`, `g.ellipse(mode, x, y, rx, ry)` | |
| `g.arc(mode, [arctype,] x, y, r, a1, a2)` | `arctype`: `"pie"` (default), `"open"`, `"closed"` |
| `g.line(x1, y1, x2, y2, ...)` or `g.line(points)` | |
| `g.polygon(mode, x1, y1, ...)` or `g.polygon(mode, points)` | |
| `g.print(text, x, y)` | One line per `\n`. Text does not rotate |
| `g.printf(text, x, y, limit, align)` | `align`: `"left"`, `"center"`, `"right"` inside width `limit` |
| `g.push()`, `g.pop()`, `g.origin()`, `g.translate(dx, dy)`, `g.rotate(a)`, `g.scale(sx[, sy])` | |
| `g.theme.foreground`, `.background`, `.muted`, `.accent`, `.border` | Theme colors as `{r, g, b, a}`; use them so drawings work in light and dark themes |

Nothing else exists (no images, shaders, `love.*`, `love.update`, or input). Calling a missing function is an error that lists the available ones.

## Rules

- Draw everything every call: `draw` starts from an empty canvas with no transform. Compute the animation from `t` (for example `local phase = t % 2`), not from state kept between calls.
- `math.random` is reset to the same seed before every draw, so the picture is the same for the same `t` (stars do not flicker). Books get the frame at `t = 0`, so make that frame meaningful.
- The script runs in a sandbox: no files, network, `io`, `os.execute`, or `require`. `os.date`, `os.time`, `os.clock`, `string`, `table`, `math`, and `utf8` work. Each draw has a 0.1 s limit and each command 2 s.
- Lua strings are bytes. For Korean or other non-ASCII text, use `utf8.len`, `utf8.codes`, `utf8.offset`, and `utf8.char` instead of `#s`, `s:sub`, or `s:upper` when counting or cutting characters.
- After writing or changing an extension, always test it with `run_extension` (draw at `t = 0` and at a later `t` if animated; run each command) and fix any error before telling the user it is done. Check that the reported drawing bounds fall inside the canvas.
- Tell the user how to use it: the ```ext block to put in a sheet, or the Extensions menu item.
