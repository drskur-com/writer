# seedling — paths, gradients, and morphing

A sprout sways in the wind under a glowing sun. The leaf, the two poses of the stem, and the gradients are built once at the top of the file and reused in every frame. `g.lerpPath` blends the two stem poses (same commands, different numbers), and `pointAt` puts the leaves on the stem's tip, turned along it. Block:

````
```ext seedling
sway: 0.8
```
````

```lua
--[[---
name: seedling
description: 바람에 흔들리는 새싹. 4초마다 되풀이한다.
args:
  sway: 흔들리는 정도(0~1). 기본값 0.6.
example:
  sway: 0.8
---]]

local W, H = 300, 200
local GROUND = 160

-- Built once at load, reused in every frame.
local LEAF = g.newPath("M0 0 C 12 -14 34 -14 46 0 C 34 10 12 10 0 0 Z")
-- Two poses of the stem with the same commands, so lerpPath can blend them.
local STILL = g.newPath():moveTo(150, GROUND):curveTo(150, 130, 150, 110, 150, 80)
local BENT = g.newPath():moveTo(150, GROUND):curveTo(150, 130, 165, 105, 185, 85)
local SKY = g.newLinearGradient(0, 0, 0, GROUND, { { 0, "#bfe3ff" }, { 1, "#f4fbff" } })
local SOIL = g.newLinearGradient(0, GROUND, 0, H, { "#8b5a2b", "#5b3a1a" })
local SUN = g.newRadialGradient(250, 45, 40, {
  { 0, { 1, 0.9, 0.4, 1 } },
  { 0.4, { 1, 0.85, 0.3, 0.6 } },
  { 1, { 1, 0.85, 0.3, 0 } },
})
local LEAF_COLOR = g.newLinearGradient(0, -8, 46, 8, { "#5fbf4a", "#2e7d32" })

return {
  size = { W, H },
  animated = true,
  period = 4,
  draw = function(g, t, args)
    local sway = math.max(0, math.min(1, args.sway or 0.6))
    g.setColor(SKY)
    g.rectangle("fill", 0, 0, W, GROUND)
    g.setColor(SUN)
    g.circle("fill", 250, 45, 40)
    g.setColor(SOIL)
    g.rectangle("fill", 0, GROUND, W, H - GROUND)

    -- 0 → 1 → 0 over one period, eased.
    local k = g.ease.inOutSine(g.ease.pingPong(t / 2)) * sway
    local stem = g.lerpPath(STILL, BENT, k)
    g.setColor(0.25, 0.55, 0.2)
    g.setLineWidth(5)
    g.setLineCap("round")
    g.stroke(stem)

    -- Two leaves on the tip, turned along the stem; the second is mirrored.
    local x, y, angle = stem:pointAt(1)
    g.setColor(LEAF_COLOR)
    for _, side in ipairs({ -1, 1 }) do
      g.push()
      g.translate(x, y)
      g.rotate(angle + side * 0.9)
      g.scale(1, side)
      g.fill(LEAF)
      g.pop()
    end
  end,
}
```
