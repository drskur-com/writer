# Drawing extensions (`draw`)

Read this before writing or changing a `draw` function. For 3D, also read `draw-3d.md`; for buttons and hover, also read `interactive.md`.

## `draw(g, t, args, ui)`

- `ui`: the mouse state for interactive drawings (`interactive.md`); an empty table otherwise, so ignore it unless you declare regions.
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
| `g.print(text, x, y[, r, sx])` | Text with its top-left at `x, y`. `\n` starts a new line. `r` turns it (radians, clockwise) around `x, y`; `sx` scales the size. Text also follows `g.rotate`. |
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
| `g.setClip(path[, rule])` / `g.setClip()` | Draw only inside a path (or SVG path string), placed with the current transform. Calling it again keeps only where all clip paths overlap; `g.setClip()` removes them all. `rule` defaults to the current fill rule. Text is clipped to the shape too. |

### Shadows, blur, and blend modes

Each applies to everything drawn after it (shapes, outlines, and text) until changed. Sizes and offsets follow the transform at the time you call them.

| Function | Meaning |
|---|---|
| `g.setShadow(color[, blur, dx, dy])` / `g.setShadow()` | A soft shadow under each shape: `color` is a table or `"#rrggbb[aa]"` (use alpha, e.g. `{0, 0, 0, 0.3}`), `blur` its softness (default 4), `dx, dy` its offset (default 0, 2). `g.setShadow()` turns it off. |
| `g.setBlur(radius)` / `g.setBlur()` | Blur the shapes themselves (glows, out-of-focus backgrounds, mist). `0` or no argument turns it off. |
| `g.setBlendMode(mode)` / `g.getBlendMode()` | How a shape mixes with what is already drawn: `"normal"` (or LÖVE's `"alpha"`, the default), `"multiply"` (shading, ink), `"screen"` and `"add"` (light, fire, glow), `"overlay"`, `"darken"`, `"lighten"`, `"colorDodge"`, `"colorBurn"`, `"hardLight"`, `"softLight"`, `"difference"`, `"exclusion"`, `"hue"`, `"saturation"`, `"color"`, `"luminosity"`. The canvas starts transparent, so draw a background first for the mix to show. |

Blur and shadows cost more to draw than plain shapes: use them on a few large shapes rather than hundreds of small ones, and turn them off right after. In PDF exports a drawing that uses blur, shadows, or `"add"` becomes a picture instead of vectors.

### Animation helpers

| Function | Meaning |
|---|---|
| `g.ease.<name>(t)` | Easing curves from 0–1 to 0–1: `linear`, `inQuad`, `outQuad`, `inOutQuad`, `inCubic`, `outCubic`, `inOutCubic`, `inSine`, `outSine`, `inOutSine`, `inBack`, `outBack`, `outElastic`, `outBounce`, and `pingPong` (0 → 1 → 0 as `t` goes 0 → 1 → 2). |
| `g.lerp(a, b, t)` | `a + (b - a) * t`. |
| `g.lerpColor(c1, c2, t)` | Mixed color as `{ r, g, b, a }`. |

### Theme

`g.theme.foreground`, `g.theme.background`, `g.theme.muted` (secondary text), `g.theme.accent`, `g.theme.border`: `{ r, g, b, a }` tables for the current theme. Pass them to `g.setColor`.

## Not available

Anything else in LÖVE: images, canvases, shaders, GPU meshes, textures, per-pixel lighting, fonts from files, `love.update`/`love.draw` callbacks, keyboard input, dragging, sound in a block. (Hover and clicking on regions are available: `interactive.md`; a running game with keyboard and mouse runs in a window: `play.md`.) Also: text along a path. Calling a function that does not exist fails with a list of the available ones.

## Design rules

- **Theme colors.** Use `g.theme.foreground`, `.muted`, `.accent`, `.border`, and `.background` for lines, text, and backgrounds so the drawing works in light and dark themes. Use fixed colors only for things that have their own color (a red target, a blue sky).
- **Size.** Set `size` to the space the drawing needs, usually 300–600 wide and 100–300 high. The app shrinks a canvas that is wider than the page and never enlarges it. When the space depends on the arguments (for example more bars), make `size` a `function(args)` returning `{ width, height }`. To make a drawing bigger or smaller, the user writes `scale: 1.5` in the block; the app handles it for every extension, so never define a `scale` argument. Lay things out from `g.getWidth()` and `g.getHeight()` rather than repeating numbers.
- **Animation.** Compute everything from `t`, for example `local phase = (t % period) / period`. `draw` keeps no state between calls and starts each call from an empty canvas. Make the frame at `t = 0` meaningful on its own: the book preview and PDF, EPUB, and DOCX exports show that frame, in the light theme. For a loop, set `period` to its length in seconds (at most 10): the app repeats `t` over that loop, so the same shapes come back and are cheaper to draw. Leave `animated` off for still pictures; an animation redraws the window 30 times a second while visible.
- **Randomness.** `math.random` restarts from the same seed on every draw, so random stars or noise stay still and look the same every time. To vary them over time, derive the values from `t`.
- **Shapes.** Draw organic shapes (leaves, hair, clouds, ribbons, characters) with paths and curves (`g.newPath`, `curveTo`, or an SVG path string) rather than many small polygons: they look smoother and cost one shape each. Build paths and gradients once at the top of the file and reuse them; place copies with `g.push`/`g.translate`/`g.rotate`/`g.pop`. Gradients (a sky, a glow with a transparent end color) give depth cheaply; `g.setShadow` lifts cards and characters off the background, and `g.setBlur` with `"add"` or `"screen"` makes glows. Cut and merge shapes with `union`/`subtract`/`intersect`, draw inside a shape with `g.setClip` (stripes on a ball, a scene seen through a window), and make borders or halos with `outline`/`offset`.
- **Motion.** Morph between two poses of the same path with `g.lerpPath(a, b, g.ease.inOutSine(k))`; move things along a curve with `path:pointAt(s)`; draw a line on with `path:trim(0, s)`; animate a dash `offset` for flowing lines. Ease motion with `g.ease.*` so it does not look mechanical.
- **Text.** `g.print` draws one line per `\n` and does not wrap. Use `g.printf(text, x, y, width, "center")` to center text. Keep labels short.

## Planning and testing a drawing

1. **Plan the picture before writing code.** Decide the canvas size, then write down for yourself the main parts with their positions and sizes in canvas units, and check the proportions against how the subject really looks (for example, an adult figure is about 7 heads tall, eyes sit halfway down the head, a cat's ears are about a third of its head width). Decide where parts overlap and which is in front, the light direction, and a small palette. Then write the code from that plan, one part at a time, with named numbers (`local headY = 80`) rather than scattered constants, so fixes are easy.
2. **Draw with `run_extension`** at `t = 0` with the arguments the block will use. If animated, pass `times` instead to see several frames in one picture (for example `[0, 0.7, 1.9]` and the end of a loop). Also call once without arguments to check the defaults, and once with the header's `example` values (the Insert dialog's preview).
3. **Always pass `goal`**: what the picture should look like, in concrete visual terms. An independent reviewer that sees only the user's request, your goal, and the picture scores each drawing from 1 to 10 and lists its problems. Fix every problem it names, starting with the first, and draw again. Passing is 7; aim higher. If you finish below 7 or without drawing your last change, the app sends you back to keep working. Do not argue with the review or call a drawing good that it scored low.
4. **Look at the attached picture critically, as the user will**: are the proportions and positions right, do parts line up and touch where they should, is anything hidden behind another shape, cut off, too small to see, or overlapping text? Does it actually look like what the user asked for? If not, fix the code and look again; repeat until it looks right, not just until it runs. Do not describe the picture to the user as good unless the image shows it.
5. The result also reports the shape count and the drawing bounds. Fix it if anything is drawn outside the canvas, if nothing is drawn, or if the shape count is in the thousands.

## Examples

Complete files in `examples/`. Read the closest one before writing a new drawing.

| File | Shows |
|---|---|
| `examples/archer.md` | An animation with `period`. |
| `examples/bar-chart.md` | A still picture whose `size` follows its arguments. |
| `examples/night-sky.md` | Stable randomness with a slow twinkle. |
| `examples/seedling.md` | Paths, gradients, and morphing. |
| `examples/route.md` | Moving along a path, drawing a line on, dashes. |
| `examples/turntable.md` | A 3D scene (with `draw-3d.md`). |
| `examples/pick-chart.md` | Hover and click (with `interactive.md`). |
