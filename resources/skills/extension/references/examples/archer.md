# archer — an animation (`animated = true`)

An archer shoots an arrow at a target every two seconds. `period` makes the app reuse the frames after the first loop. Block:

````
```ext archer
speed: 1.5
label: 명중!
```
````

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

local PERIOD = 2

return {
  size = { 480, 160 },
  animated = true,
  period = PERIOD,
  draw = function(g, t, args)
    local speed = args.speed or 1
    local label = args.label or "Hit!"
    local w, h = g.getDimensions()
    local ground = h - 20
    local phase = (t % PERIOD) / PERIOD

    -- ground
    g.setColor(g.theme.border)
    g.line(10, ground, w - 10, ground)

    -- archer: a stick figure
    g.setColor(g.theme.foreground)
    g.setLineWidth(2)
    g.circle("line", 60, ground - 78, 10)
    g.line(60, ground - 68, 60, ground - 30)
    g.line(60, ground - 30, 48, ground)
    g.line(60, ground - 30, 72, ground)
    g.line(60, ground - 55, 82, ground - 58)

    -- bow
    g.setColor(g.theme.accent)
    g.arc("line", "open", 78, ground - 58, 22, -1.2, 1.2)

    -- target
    local tx, ty = w - 50, ground - 55
    for i, radius in ipairs({ 26, 17, 8 }) do
      if i % 2 == 1 then g.setColor(0.85, 0.2, 0.2) else g.setColor(1, 1, 1) end
      g.circle("fill", tx, ty, radius)
    end

    -- arrow: flies from the bow to the target, then stays there for the rest of the loop
    local progress = math.min(phase * speed * 2, 1)
    local x = 100 + (tx - 100) * progress
    g.setColor(g.theme.foreground)
    g.line(x - 40, ty - 3, x, ty - 3)
    g.polygon("fill", x, ty - 3, x - 8, ty - 7, x - 8, ty + 1)

    if progress >= 1 then
      g.setColor(g.theme.accent)
      g.setFont(16)
      g.printf(label, tx - 100, 16, 200, "center")
    end
  end,
}
```
