# Writer extension API

## The file

`Extensions/<name>.lua` (`<name>`: letters, digits, `-`, `_`) runs once when it loads and must `return` a table:

| Field | Type | Meaning |
|---|---|---|
| `size` | `{ width, height }` or `function(args)` returning it | Canvas size in canvas units (1–4096). Default `{ 400, 200 }`. Shown at this size, or shrunk to the page width. A function lets each block choose its size from its arguments (asked once per argument set; must finish within 0.1 s). `g.getWidth()`/`g.getHeight()` in `draw` return the size for that block. |
| `animated` | boolean | `true`: redraw while the block is visible, 30 times a second. Default `false`: draw once at `t = 0`. |
| `period` | number | Seconds after which an animation repeats exactly (0–10). The app keeps the frames of one period and stops calling `draw` after the first loop, so set it for every looping animation. |
| `draw` | `function(g, t, args)` | Draws the block. |
| `commands` | list of `{ title = string, run = function(ctx) }` | Items in the Extensions menu, in this order. |

The table needs `draw`, `commands`, or both. Other fields are ignored. Code outside the returned functions (helpers, constants) runs once at load; keep it free of side effects. The canvas `g` is also a global there, so build paths and gradients once at the top of the file (`local LEAF = g.newPath(...)`) and reuse them in every `draw`; drawing at load time does nothing.

## The header

Every extension file starts with a header: a Lua block comment from `--[[---` to `---]]` holding `key: value` lines. Lua ignores it; the app reads it without running the script. The Insert → Extension… dialog lists extensions by it (with a live preview), and the assistant's prompt lists them so existing extensions get reused.

```lua
--[[---
name: archer
description: 궁수가 과녁에 화살을 쏘는 애니메이션. 2초마다 되풀이한다.
args:
  speed: 화살 속도. 기본값 1.
  label: 맞혔을 때 보이는 글. 기본값 "Hit!".
example:
  speed: 1.5
  label: 명중!
---]]
```

| Key | Meaning |
|---|---|
| `name` | The extension name. Informational: blocks always use the file name. |
| `description` | One line saying what it draws or does. **Write it in the user's language** (it is shown to the user), unlike code and comments. |
| `args` | Indented `key: description` lines, one per block argument, with its default. Also in the user's language. Leave out for command-only extensions. |
| `example` | Indented `key: value` lines: the starting values in the Insert dialog, where the user can change them and watch the preview before inserting the block. Pick values that show the drawing well. |

The header must be the first thing in the file (blank lines before it are fine). Values are single lines.

## The block

````
```ext <name>
key: value
other: value
```
````

- `` ~~~ `` fences and longer fences work too. A fence without its closing line is not a block yet.
- Each body line `key: value` becomes `args.key`. A value that reads as a number becomes a number, `true`/`false` become booleans, and anything else stays a string (spaces at the ends removed). Lines without `:` are ignored.
- `scale` is reserved for the app: `scale: 1.5` shows the whole canvas 1.5 times larger (0.1–4; still shrunk to the page width). It works for every extension and is not passed to `draw` or `size`, so do not use `scale` as an argument name or draw bigger yourself for it.
- The block is ordinary Markdown: other apps show it as a code block.

## `draw(g, t, args)`

- `t`: seconds since the block appeared (always `0` when not animated). Frames are 1/30 s apart. With `period`, `t` runs from 0 to `period` and repeats.
- Each call starts with an empty canvas, no transform, line width 1, `"miter"` joins, `"butt"` caps, no dash, no scissor, the `"nonzero"` fill rule, font size 12, and the theme's text color.
- `draw` runs on its own thread, so a slow `draw` makes only its own block update less often; it never stops the editor. Still keep a frame under about 10 ms.
- Limits per call: 0.1 s; 20,000 shapes for a still picture, 2,000 per frame for an animation.
- Code outside the returned functions runs once when the file loads and must finish within 0.1 s: keep it to constants and helper functions.

## Canvas `g`

The functions follow LÖVE's `love.graphics`. Both `g.circle(...)` and `g:circle(...)` work.

Coordinates are canvas units: origin at the top-left, x right, y down. Angles are radians, clockwise on screen (y is down).

Colors: `r, g, b[, a]` from 0 to 1, a table `{ r, g, b[, a] }`, or a string `"#rrggbb"` / `"#rrggbbaa"`. If any of r, g, b is above 1, all are read as 0–255.

### State

| Function | Meaning |
|---|---|
| `g.setColor(color)` | Color for everything drawn after it. |
| `g.getColor()` | Returns `r, g, b, a` (0–1). |
| `g.setLineWidth(width)` / `g.getLineWidth()` | Width of `"line"` shapes. Scales with `g.scale`. |
| `g.setFont(size)` | Font size for `print`/`printf` (default 12). |
| `g.newFont(size)` | Returns `size`, so `g.setFont(g.newFont(20))` works as in LÖVE. |
| `g.getWidth()`, `g.getHeight()`, `g.getDimensions()` | Canvas size. |

### Shapes

`mode` is `"fill"` or `"line"`.

| Function | Meaning |
|---|---|
| `g.clear([color])` | Fills the whole canvas (ignores the transform). Without a color, uses the current color. |
| `g.rectangle(mode, x, y, w, h[, rx[, ry]])` | Rectangle; `rx`, `ry` round the corners. |
| `g.circle(mode, x, y, radius)` | |
| `g.ellipse(mode, x, y, rx[, ry])` | |
| `g.arc(mode, [arctype,] x, y, radius, angle1, angle2)` | `arctype`: `"pie"` (default, lines to the center), `"open"`, `"closed"` (chord). From `angle1` to `angle2`. |
| `g.line(x1, y1, x2, y2, ...)` / `g.line({ x1, y1, ... })` | Open polyline, at least two points. |
| `g.polygon(mode, x1, y1, ...)` / `g.polygon(mode, { ... })` | Closed shape, at least three points. |

### Text

| Function | Meaning |
|---|---|
| `g.print(text, x, y[, r, sx])` | Text with its top-left at `x, y`. `\n` starts a new line. `sx` scales the size; `r` is ignored (text does not rotate). |
| `g.printf(text, x, y, limit, align)` | Aligns each line inside the width `limit`: `"left"`, `"center"`, `"right"`. Does not wrap long lines. |

Text uses the app's interface font. Line height is 1.25 × the font size.

### Transform

| Function | Meaning |
|---|---|
| `g.push()` / `g.pop()` | Save and restore the transform (up to 64 deep). |
| `g.origin()` | Reset the transform. |
| `g.translate(dx, dy)`, `g.rotate(angle)`, `g.scale(sx[, sy])` | Apply to everything drawn after it, as in LÖVE (the last call applies first to points). |

### Paths

Beyond LÖVE: vector paths with curves. Build a path once (at the top of the file) and draw it as often as you like; each draw uses the current color, line style, and transform.

| Function | Meaning |
|---|---|
| `g.newPath([svg])` | An empty path, or one read from an SVG path string (`d` attribute): `M L H V C S Q T A Z` and their lowercase relative forms. |
| `g.fill(path)` / `g.stroke(path)` | Fill or outline it. A path string works too: `g.fill("M0 0 L 10 0 L 5 8 Z")`. |
| `g.lerpPath(a, b, t)` | The path between `a` (`t = 0`) and `b` (`t = 1`). Both need the same commands in the same order (build them with the same calls and change only the numbers). Use it to morph shapes and animate control points. |

Building (each returns the path, so calls chain: `g.newPath():moveTo(0, 0):lineTo(10, 0):close()`):

| Method | Meaning |
|---|---|
| `p:moveTo(x, y)` | Start a new subpath. |
| `p:lineTo(x, y)` | |
| `p:curveTo(c1x, c1y, c2x, c2y, x, y)` | Cubic Bézier. |
| `p:quadTo(cx, cy, x, y)` | Quadratic Bézier. |
| `p:arc(x, y, r, a1, a2)` | Circular arc around `x, y` from `a1` to `a2` (radians, clockwise on screen); joined to the current point with a line. |
| `p:rect(x, y, w, h[, rx, ry])`, `p:circle(x, y, r)`, `p:ellipse(x, y, rx[, ry])`, `p:polygon(x1, y1, ...)` | Add a closed shape. |
| `p:svg(d)` | Append an SVG path string. |
| `p:close()` | Close the subpath back to its start. |

Reading and changing (these return numbers or a **new** path; the original stays as it is):

| Method | Meaning |
|---|---|
| `p:length()` | Length along the path. |
| `p:pointAt(s)` | `x, y, angle` at fraction `s` (0–1) of the length; `angle` is the direction of travel. Move things along a path with it (`g.translate(x, y); g.rotate(angle)`). |
| `p:trim(s1, s2)` | The part from fraction `s1` to `s2`. `p:trim(0, s)` with a growing `s` draws a line as if by hand. Curves stay curves. |
| `p:bounds()` | `x, y, w, h`. |
| `p:points()` / `p:withPoints(list)` | All anchor and control points as one list `{ x1, y1, x2, y2, ... }`, and the same path with other points (same count). Animate single control points with them. |
| `p:lerp(other, t)` | Same as `g.lerpPath(p, other, t)`. |
| `p:translate(dx, dy)`, `p:rotate(angle[, cx, cy])`, `p:scale(sx[, sy, cx, cy])` | Moved, turned around `cx, cy`, or scaled from `cx, cy` (default origin). |
| `p:clone()` | A copy you can keep building on. |

Combining (each returns a **new** path; `other` is a path or an SVG path string; `rule` is `"nonzero"` (default) or `"evenodd"` and says how both paths are filled):

| Method | Meaning |
|---|---|
| `p:union(other[, rule])` | Both shapes merged into one outline. |
| `p:subtract(other[, rule])` | `p` with `other` cut out (a bite out of an apple, a window in a wall). |
| `p:intersect(other[, rule])` | Only where both overlap. |
| `p:xor(other[, rule])` | Where exactly one of them is. |
| `p:outline(width[, join, cap, miterLimit])` | The outline of the line you would get with `g.stroke(p)` at that width, as a path to fill: give it a gradient, combine it, or outline it again (a lettering-style double outline). `join`: `"miter"` (default), `"round"`, `"bevel"`; `cap`: `"butt"` (default), `"round"`, `"square"`. |
| `p:offset(distance[, join, rule])` | The filled shape grown by `distance` (negative shrinks). Good for halos, borders around a shape, and insets. `join` defaults to `"round"`. |

The results are polygons: their curves become many short straight lines (fine to look at, but no longer the same commands, so `lerpPath` cannot morph them into the original). Combine once at the top of the file when the inputs do not change.

### Gradients

`g.newLinearGradient(x1, y1, x2, y2, stops)` and `g.newRadialGradient(cx, cy, r, stops)` return a gradient; pass it to `g.setColor` and every fill and outline after it uses it (text uses its first color). `stops` is `{ {0, color}, {0.6, color}, {1, color} }` with positions from 0 to 1, or just `{ color, color, ... }` spread evenly. Colors take any color form, including alpha: `{ 1, 1, 1, 0 }` fades out. Outside the range the end colors continue.

Gradient coordinates follow the transform at the time you draw, like a shape's. Make a gradient once at the top of the file when it does not change.

### Line style, fill rule, and clipping

| Function | Meaning |
|---|---|
| `g.setLineJoin(join)` | `"miter"` (default), `"round"`, `"bevel"`. |
| `g.setLineCap(cap)` | `"butt"` (default), `"round"`, `"square"`. |
| `g.setMiterLimit(n)` | How far sharp miter joins may reach (default 10). |
| `g.setDash({on, off, ...}[, offset])` / `g.setDash()` | Dashed outlines; lengths follow the transform. Change `offset` with `t` for "marching ants". `g.setDash()` returns to solid lines. |
| `g.setFillRule(rule)` | `"nonzero"` (default) or `"evenodd"`. With `"evenodd"`, an inner subpath cuts a hole (a ring, the letter O) regardless of its direction. |
| `g.setScissor(x, y, w, h)` / `g.setScissor()` | Draw only inside that canvas rectangle (ignores the transform), as in LÖVE. |
| `g.setClip(path[, rule])` / `g.setClip()` | Draw only inside a path (or SVG path string), placed with the current transform. Calling it again keeps only where all clip paths overlap; `g.setClip()` removes them all. `rule` defaults to the current fill rule. Text is clipped to the clip area's bounding rectangle, not to its shape. |

### Animation helpers

| Function | Meaning |
|---|---|
| `g.ease.<name>(t)` | Easing curves from 0–1 to 0–1: `linear`, `inQuad`, `outQuad`, `inOutQuad`, `inCubic`, `outCubic`, `inOutCubic`, `inSine`, `outSine`, `inOutSine`, `inBack`, `outBack`, `outElastic`, `outBounce`, and `pingPong` (0 → 1 → 0 as `t` goes 0 → 1 → 2). |
| `g.lerp(a, b, t)` | `a + (b - a) * t`. |
| `g.lerpColor(c1, c2, t)` | Mixed color as `{ r, g, b, a }`. |

### Theme

`g.theme.foreground`, `g.theme.background`, `g.theme.muted` (secondary text), `g.theme.accent`, `g.theme.border`: `{ r, g, b, a }` tables for the current theme. Pass them to `g.setColor`.

### 3D scenes

`g.newScene([x, y, w, h])` returns a 3D scene that draws into that rectangle of the canvas (default: the whole canvas). Collect solids, lines, and faces in it, then call `scene:draw()` once: the app projects them through the camera and adds them to the canvas as ordinary 2D shapes, farthest first. Call scene functions with a colon (`s:box(...)`).

Coordinates: right-handed, **y up**, in any unit you like (sizes around 1 work well with the default camera). Angles are radians. Solids are centered on the origin; move them with the transform. Each face gets one flat shade from the light (ambient + direct).

| Function | Meaning |
|---|---|
| `s:camera(ex, ey, ez[, tx, ty, tz[, fov]])` | Camera at `e` looking at `t` (default origin), y up. `fov`: vertical field of view in degrees (default 45). Default camera: `3, 2, 4` looking at the origin. |
| `s:ortho(height)` | No perspective: shows `height` scene units from top to bottom of the scene (good for isometric pictures). `s:camera` with a `fov` switches back. |
| `s:light(x, y, z[, ambient])` | Direction the light comes **from** (default `0.4, 1, 0.7`, upper right front) and ambient light 0–1 (default 0.35). |
| `s:setColor(color)` | Color for what is added after it (same forms as `g.setColor`). Starts as the canvas's current color. |
| `s:push()`, `s:pop()`, `s:origin()` | Save, restore, reset the 3D transform (up to 64 deep). |
| `s:translate(x, y, z)`, `s:scale(sx[, sy, sz])` | |
| `s:rotate(angle, ax, ay, az)`, `s:rotateX(a)`, `s:rotateY(a)`, `s:rotateZ(a)` | Turn around an axis, counterclockwise when the axis points at you. The last call applies first to points, as in 2D. |
| `s:box(mode, w[, h, d])` | Box; `h`, `d` default to `w` (a cube). |
| `s:sphere(mode, r[, segments])` | `segments` around (3–64, default 16). |
| `s:cylinder(mode, r, h[, segments])`, `s:cone(mode, r, h[, segments])` | Upright along y, `segments` default 24. The cone's tip points up. |
| `s:torus(mode, R, r[, segments, sides])` | Ring around the y axis: `R` to the tube's center, tube radius `r`. |
| `s:plane(mode, w[, d])` | Flat floor at y = 0, `w` along x and `d` along z. Visible from both sides. |
| `s:polygon(mode, x1, y1, z1, ...)` | One flat face (points also as one table). Visible from both sides. |
| `s:mesh(mode, vertices, faces)` | Your own model: `vertices = { {x, y, z}, ... }`, `faces = { {1, 2, 3}, {1, 3, 4}, ... }` (vertex numbers start at 1). Faces are visible from both sides. |
| `s:line(x1, y1, z1, x2, y2, z2, ...)` | 3D polyline in the current color and `g.setLineWidth`. |
| `s:project(x, y, z)` | Returns `x, y, depth`: where that point (with the current transform) lands on the canvas, or `nil` behind the camera. Use it to place `g.print` labels. |
| `s:draw()` | Draws everything collected, with the current 2D transform of `g`. |

- `"fill"` faces are shaded; `"line"` draws each face's outline in the current color, without shading, including hidden edges.
- Hidden faces of boxes, spheres, cylinders, cones, and tori are skipped. Overlaps are resolved by sorting faces by their average distance (painter's algorithm), not per pixel: faces that cut through each other, or a long face next to small ones, can be drawn in the wrong order. Keep objects separate and split big faces (a floor) into a few if objects stand on them and look wrong.
- Every visible face is one shape: keep the shape limit in mind (an animation frame allows 2,000; a 16-segment sphere is about 70 visible faces). Use fewer segments for small or many objects.
- Text and `g` shapes are not part of the scene: draw them before `s:draw()` to put them behind, after to put them in front.

### Not available

Anything else in LÖVE: images, canvases, shaders, GPU meshes, textures, per-pixel lighting, fonts from files, `love.update`/`love.draw` callbacks, input, sound. Also: blend modes, blur and soft shadows (layer a few translucent copies, or `p:offset` with fading colors), text along a path, clipping text to a shape. Calling a function that does not exist fails with a list of the available ones.

## Commands

`run(ctx)` gets:

| Field | Meaning |
|---|---|
| `ctx.selection` | Selected text (empty when nothing is selected). |
| `ctx.text` | The whole sheet as the editor holds it: one line per paragraph. |
| `ctx.sheet` | The sheet's path from the library root. |

Return a string (or number) to replace the selection, or insert it at the cursor when nothing is selected. Return `nil` to change nothing. Calling `error("message")` shows the message to the user. Limit: 2 s.

## The sandbox

Available: `string` (including Luau's `string.split`), `table`, `math`, `utf8`, `bit32`, `os.date`, `os.time`, `os.clock`, `os.difftime`, `print` (ignored), `pcall`, `error`, `tostring`, `tonumber`, `pairs`, `ipairs`, `select`, `type`, `setmetatable`.

Not available: files (`io`), processes (`os.execute`), the network, other modules (`require`), changing globals of the app. Memory is limited to 64 MB.

`math.random` restarts from the same seed before every `draw`, so the same `t` always gives the same picture.

## Strings and Korean

Lua strings are bytes; Korean characters take three bytes each. `#s`, `s:sub(i, j)`, `s:reverse()`, and `s:len()` count and cut bytes. To work with characters:

```lua
local count = utf8.len(s)                    -- number of characters
local chars = {}
for _, code in utf8.codes(s) do              -- each character
  table.insert(chars, utf8.char(code))
end
local first3 = s:sub(1, (utf8.offset(s, 4) or #s + 1) - 1)   -- first 3 characters
```

`string.find`, `string.gsub`, and `string.split` with plain ASCII patterns or literal Korean text work. Lua patterns are not regular expressions (`%a`, `%w` match only ASCII letters).
