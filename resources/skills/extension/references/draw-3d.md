# 3D scenes (`g.newScene`)

Part of the drawing API (`draw.md`). Example: `examples/turntable.md`.

`g.newScene([x, y, w, h])` returns a 3D scene that draws into that rectangle of the canvas (default: the whole canvas). Collect solids, lines, and faces in it, then call `scene:draw()` once: the app projects them through the camera and draws them per pixel with a depth buffer, so whatever is nearer hides what is behind it, even where objects cut through each other. The whole scene is one shape on the canvas. Call scene functions with a colon (`s:box(...)`).

Coordinates: right-handed, **y up**, in any unit you like (sizes around 1 work well with the default camera). Angles are radians. Solids are centered on the origin; move them with the transform. Light (ambient + direct) is computed per pixel. Curved solids (spheres, cylinders, cones, tori) are shaded smoothly; boxes, planes, polygons, and meshes have flat faces.

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
| `s:line(x1, y1, z1, x2, y2, z2, ...)` | 3D polyline in the current color and `g.setLineWidth`, hidden where solids are in front of it. |
| `s:setShading(mode)` | `"smooth"` (default) or `"flat"`: how curved solids added after it are shaded. `"flat"` gives a faceted, low-poly look. |
| `s:project(x, y, z)` | Returns `x, y, depth`: where that point (with the current transform) lands on the canvas, or `nil` behind the camera. Use it to place `g.print` labels. |
| `s:draw()` | Draws everything collected, with the current 2D transform of `g`. |

- `"fill"` faces are shaded; `"line"` draws each face's outline in the current color, without shading (edges behind filled solids are hidden; a scene of only outlines shows them all).
- Translucent colors (alpha below 1) are drawn after the opaque ones, farthest first, so glass and water show what is behind them. The back faces of closed solids are skipped, so a translucent sphere shows only its front.
- Two faces in exactly the same place (the tops of two boxes at the same height) flicker between each other; offset one slightly.
- The scene counts as one shape, so the 2,000-shape limit does not apply to it; it may hold up to 20,000 faces. Cost grows with the area the scene covers more than with its faces: a full-canvas animated scene takes a few milliseconds per frame.
- Text and `g` shapes are not part of the scene: draw them before `s:draw()` to put them behind, after to put them in front.
