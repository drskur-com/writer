# 3D scenes (`g.newScene`)

Part of the drawing API (`draw.md`). Example: `examples/turntable.md`.

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
