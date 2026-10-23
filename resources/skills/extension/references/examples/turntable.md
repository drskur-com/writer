# turntable — a 3D scene (`g.newScene`)

A box, a ball, and a cone on a floor, seen by a camera that circles them once every 8 seconds. Block:

````
```ext turntable
speed: 1
label: 3D
```
````

```lua
--[[---
name: turntable
description: 바닥 위의 상자·공·원뿔을 카메라가 한 바퀴 돌며 보여 주는 3D 애니메이션.
args:
  speed: 도는 속도(8초에 몇 바퀴). 기본값 1.
  label: 공 위에 보이는 글. 기본값 없음.
example:
  speed: 1
  label: 3D
---]]

local PERIOD = 8

return {
  size = { 400, 240 },
  animated = true,
  period = PERIOD,
  draw = function(g, t, args)
    local speed = math.max(1, math.floor(args.speed or 1))
    local angle = (t / PERIOD) * speed * 2 * math.pi

    local s = g.newScene()
    s:camera(5 * math.cos(angle), 2.5, 5 * math.sin(angle), 0, 0.5, 0, 40)

    s:setColor(g.theme.border)
    s:plane("fill", 5, 5)

    s:push()
    s:translate(-1.2, 0.5, 0)
    s:rotateY(0.4)
    s:setColor(g.theme.accent)
    s:box("fill", 1)
    s:pop()

    s:push()
    s:translate(0.6, 0.6, -0.6)
    s:setColor(0.85, 0.3, 0.3)
    s:sphere("fill", 0.6, 20)
    s:pop()

    s:push()
    s:translate(0.6, 0.5, 1.1)
    s:setColor(0.3, 0.7, 0.4)
    s:cone("fill", 0.45, 1)
    s:pop()

    s:draw()

    -- a 2D label that follows a 3D point
    if args.label then
      local x, y = s:project(0.6, 1.5, -0.6)
      if x then
        g.setColor(g.theme.foreground)
        g.printf(args.label, x - 50, y - 8, 100, "center")
      end
    end
  end,
}
```
