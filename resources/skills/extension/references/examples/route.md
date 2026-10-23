# route — moving along a path, drawing a line on, dashes

A plane flies along a curved route. The route is an SVG path string. `trim` draws the part already flown, `pointAt` places and turns the plane, and the dash offset follows `t` so the dashed route flows. Block:

````
```ext route
from: 서울
to: 제주
```
````

```lua
--[[---
name: route
description: 점선 항로를 따라 날아가는 비행기. 3초마다 되풀이한다.
args:
  from: 출발지 이름. 기본값 "Seoul".
  to: 도착지 이름. 기본값 "Jeju".
example:
  from: 서울
  to: 제주
---]]

local W, H = 400, 160
local ROUTE = g.newPath("M40 120 C 120 20, 280 20, 360 110")
local PLANE = g.newPath("M10 0 L -8 -7 L -4 0 L -8 7 Z")

return {
  size = { W, H },
  animated = true,
  period = 3,
  draw = function(g, t, args)
    -- Fly for 2.5 s, then wait at the end.
    local s = g.ease.inOutCubic(math.min(t / 2.5, 1))

    -- The whole route, dashed; the offset moves by one dash pattern (12) per second.
    g.setColor(g.theme.muted)
    g.setLineWidth(2)
    g.setDash({ 6, 6 }, -t * 12)
    g.stroke(ROUTE)
    g.setDash()

    -- The part already flown.
    g.setColor(g.theme.accent)
    g.setLineWidth(3)
    g.setLineCap("round")
    g.stroke(ROUTE:trim(0, s))

    -- End points and their names.
    local ends = { { 40, 120, args.from or "Seoul" }, { 360, 110, args.to or "Jeju" } }
    g.setColor(g.theme.foreground)
    for _, e in ipairs(ends) do
      g.circle("fill", e[1], e[2], 4)
      g.printf(tostring(e[3]), e[1] - 40, e[2] + 10, 80, "center")
    end

    -- The plane, turned along the route.
    local x, y, angle = ROUTE:pointAt(s)
    g.push()
    g.translate(x, y)
    g.rotate(angle)
    g.setColor(g.theme.accent)
    g.fill(PLANE)
    g.pop()
  end,
}
```
