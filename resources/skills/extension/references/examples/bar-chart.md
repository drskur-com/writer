# bar-chart — a still picture whose width follows its arguments

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
