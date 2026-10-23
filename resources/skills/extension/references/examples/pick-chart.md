# pick-chart — hover and click (`g.region`, `ui`, `click`)

A bar chart: the bar under the pointer lights up, and clicking a bar selects it, which writes `selected: <n>` into the block (clicking it again clears it). The selected bar's value is shown at the top. Block:

````
```ext pick-chart
values: 12, 30, 21, 8
labels: 봄, 여름, 가을, 겨울
selected: 2
```
````

```lua
--[[---
name: pick-chart
description: 막대를 누르면 그 막대의 값을 위에 크게 보여 주는 막대 차트. 포인터를 올린 막대는 밝아진다.
args:
  values: 쉼표로 나눈 값. 기본값 "3, 5, 2, 6".
  labels: 쉼표로 나눈 이름. 기본값은 없음.
  selected: 고른 막대 번호(1부터). 막대를 누르면 바뀐다. 기본값은 없음.
example:
  values: 12, 30, 21, 8
  labels: 봄, 여름, 가을, 겨울
  selected: 2
---]]

local W, H = 360, 200
local TOP, BOTTOM, SIDE = 40, 28, 16

local function numbers(text)
  local out = {}
  for _, item in ipairs(string.split(text, ",")) do
    local n = tonumber(item)
    if n then table.insert(out, n) end
  end
  return out
end

local function trim(s)
  return (s or ""):match("^%s*(.-)%s*$")
end

return {
  size = { W, H },
  draw = function(g, t, args, ui)
    local values = numbers(args.values or "3, 5, 2, 6")
    local labels = string.split(args.labels or "", ",")
    local selected = tonumber(args.selected)
    local max = 0
    for _, v in ipairs(values) do max = math.max(max, v) end
    if #values == 0 or max <= 0 then
      g.setColor(g.theme.muted)
      g.printf("값이 없어요", 0, H / 2 - 8, W, "center")
      return
    end

    local slot = (W - SIDE * 2) / #values
    local barW = slot * 0.6
    local baseY = H - BOTTOM
    for i, v in ipairs(values) do
      local id = "bar" .. i
      local slotX = SIDE + slot * (i - 1)
      local x = slotX + (slot - barW) / 2
      local h = (baseY - TOP) * v / max
      if ui.pressed == id then h = h * 0.97 end
      local color = g.theme.muted
      if i == selected then
        color = g.theme.accent
      elseif ui.hover == id then
        color = g.lerpColor(g.theme.muted, g.theme.accent, 0.5)
      end
      g.setColor(color)
      g.rectangle("fill", x, baseY - h, barW, h, 3)
      -- The whole column reacts, so short bars are easy to hit.
      g.region(id, slotX, TOP, slot, baseY - TOP)
      g.setColor(g.theme.foreground)
      g.printf(trim(labels[i]), slotX, baseY + 6, slot, "center")
    end

    g.setFont(16)
    if selected and values[selected] then
      local name = trim(labels[selected])
      if name == "" then name = "#" .. selected end
      g.setColor(g.theme.foreground)
      g.printf(name .. ": " .. values[selected], 0, 10, W, "center")
    else
      g.setColor(g.theme.muted)
      g.printf("막대를 눌러 보세요", 0, 10, W, "center")
    end
  end,
  click = function(id, args)
    local i = tonumber(id:match("^bar(%d+)$"))
    if not i then return nil end
    if tonumber(args.selected) == i then
      return { selected = "" }
    end
    return { selected = i }
  end,
}
```

`click` keeps nothing between calls: the selection lives in the block as `selected`, so it is saved with the sheet, undo reverts it, and exports show the selected bar (without hover).
