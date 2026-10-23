# night-sky — random but stable, with a slow twinkle

`math.random` gives the same stars on every draw. The twinkle comes from `t`, not from random numbers. Block: ```` ```ext night-sky ```` with an optional `stars: 80`.

```lua
--[[---
name: night-sky
description: 별이 반짝이는 밤하늘과 초승달.
args:
  stars: 별 개수(최대 500). 기본값 60.
example:
  stars: 80
---]]

return {
  size = { 480, 180 },
  animated = true,
  draw = function(g, t, args)
    local w, h = g.getDimensions()
    g.clear(0.05, 0.07, 0.15)
    for i = 1, math.min(args.stars or 60, 500) do
      local x, y = 3 + math.random() * (w - 6), 3 + math.random() * (h - 6)
      local size = 0.5 + math.random() * 1.5
      local twinkle = 0.6 + 0.4 * math.sin(t * 2 + i)
      g.setColor(1, 1, 0.9, twinkle)
      g.circle("fill", x, y, size)
    end
    g.setColor(0.95, 0.93, 0.8)
    g.circle("fill", w - 70, 50, 24)
    g.setColor(0.05, 0.07, 0.15)
    g.circle("fill", w - 60, 44, 22)
  end,
}
```
