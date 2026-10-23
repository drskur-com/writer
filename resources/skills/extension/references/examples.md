# Extension examples

Each example is a complete `Extensions/<name>.lua` file. Start from the closest one and change it. All of them start with a header, use theme colors, and give every argument a default. The header descriptions here are in Korean because these examples were written for a Korean user; write yours in the user's language.

## archer — an animation (`animated = true`)

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

## bar-chart — a still picture whose width follows its arguments

Block:

````
```ext bar-chart
values: 12, 30, 21, 8
labels: 봄, 여름, 가을, 겨울
title: 계절별 원고 매수
```
````

```lua
--[[---
name: bar-chart
description: 막대 그래프. 값과 이름을 쉼표로 나눠 적는다.
args:
  values: 막대 값(쉼표로 나눈 숫자). 기본값 "3, 5, 2".
  labels: 막대 아래 이름(쉼표로 나눔).
  title: 위에 보이는 제목.
example:
  values: 12, 30, 21, 8
  labels: 봄, 여름, 가을, 겨울
  title: 계절별 원고 매수
---]]

local function list(text)
  local items = {}
  for _, item in ipairs(string.split(text or "", ",")) do
    item = item:match("^%s*(.-)%s*$")
    if item ~= "" then table.insert(items, item) end
  end
  return items
end

local function numbers(args)
  local values = {}
  for _, item in ipairs(list(tostring(args.values or "3, 5, 2"))) do
    table.insert(values, tonumber(item) or 0)
  end
  return values
end

return {
  -- The chart gets wider with more bars: 80 units per bar, at least 240.
  size = function(args)
    return { math.max(240, #numbers(args) * 80 + 40), 240 }
  end,
  draw = function(g, t, args)
    local w, h = g.getDimensions()
    local values = numbers(args)
    local labels = list(tostring(args.labels or ""))
    local top, bottom, left, right = 40, h - 30, 20, w - 20

    local max = 0
    for _, v in ipairs(values) do max = math.max(max, v) end
    if max == 0 then max = 1 end

    g.setFont(14)
    g.setColor(g.theme.foreground)
    g.printf(args.title or "", 0, 8, w, "center")

    local slot = (right - left) / math.max(#values, 1)
    g.setFont(11)
    for i, v in ipairs(values) do
      local barH = (bottom - top) * v / max
      local x = left + slot * (i - 1) + slot * 0.2
      g.setColor(g.theme.accent)
      g.rectangle("fill", x, bottom - barH, slot * 0.6, barH, 3)
      g.setColor(g.theme.muted)
      g.printf(tostring(v), x, bottom - barH - 16, slot * 0.6, "center")
      g.printf(labels[i] or "", x - slot * 0.2, bottom + 6, slot, "center")
    end

    g.setColor(g.theme.border)
    g.line(left, bottom, right, bottom)
  end,
}
```

## night-sky — random but stable, with a slow twinkle

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

## text-tools — commands on the selection

```lua
--[[---
name: text-tools
description: 선택한 글을 다루는 명령: 오늘 날짜 넣기, 「」로 감싸기, 글자 순서 뒤집기, 줄 정렬.
---]]

local function chars(s)
  local out = {}
  for _, code in utf8.codes(s) do table.insert(out, utf8.char(code)) end
  return out
end

return {
  commands = {
    {
      title = "오늘 날짜 넣기",
      run = function(ctx)
        local t = os.date("*t")
        return string.format("%d년 %d월 %d일", t.year, t.month, t.day)
      end,
    },
    {
      title = "「」로 감싸기",
      run = function(ctx)
        return "「" .. ctx.selection .. "」"
      end,
    },
    {
      title = "글자 순서 뒤집기",
      run = function(ctx)
        if ctx.selection == "" then return nil end
        local list = chars(ctx.selection)
        local reversed = {}
        for i = #list, 1, -1 do table.insert(reversed, list[i]) end
        return table.concat(reversed)
      end,
    },
    {
      title = "줄 정렬",
      run = function(ctx)
        if ctx.selection == "" then return nil end
        local lines = string.split(ctx.selection, "\n")
        table.sort(lines)
        return table.concat(lines, "\n")
      end,
    },
  },
}
```

`ctx.selection` can be empty: "「」로 감싸기" then inserts `「」` at the cursor, and the other two return `nil` (nothing changes).

## turntable — a 3D scene (`g.newScene`)

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

## seedling — paths, gradients, and morphing

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

## route — moving along a path, drawing a line on, dashes

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
