# chart-remote — buttons that drive another block (`set` action)

Previous/next buttons that step the `selected` bar of a `pick-chart` block named by `target` (its `id:`). The remote keeps the current index in its own `selected` too, because it cannot read the chart's arguments. Blocks:

````
```ext chart-remote
target: chart
count: 4
```

```ext pick-chart
id: chart
values: 12, 30, 21, 8
labels: 봄, 여름, 가을, 겨울
```
````

```lua
--[[---
name: chart-remote
description: 다른 차트 블록의 막대를 앞뒤로 고르는 버튼. 차트 블록에 id를 붙이고 그 이름을 target에 쓴다.
args:
  target: 고를 차트 블록의 id. 기본값 "chart".
  count: 막대 수. 기본값 4.
  selected: 지금 고른 막대 번호(1부터). 버튼을 누르면 바뀐다. 기본값 없음.
example:
  target: chart
  count: 4
actions: set
---]]

local W, H = 240, 48
local BUTTON_W, BUTTON_H = 64, 32

local function button(g, ui, id, x, label)
  local y = (H - BUTTON_H) / 2
  local fill = g.theme.border
  if ui.pressed == id then
    fill = g.theme.muted
  elseif ui.hover == id then
    fill = g.lerpColor(g.theme.border, g.theme.accent, 0.4)
  end
  g.setColor(fill)
  g.rectangle("fill", x, y, BUTTON_W, BUTTON_H, 6)
  g.setColor(g.theme.foreground)
  g.setFont(16)
  g.printf(label, x, y + 6, BUTTON_W, "center")
  g.region(id, x, y, BUTTON_W, BUTTON_H)
end

return {
  size = { W, H },
  draw = function(g, t, args, ui)
    local count = math.max(1, math.floor(tonumber(args.count) or 4))
    local selected = tonumber(args.selected)
    button(g, ui, "prev", 0, "◀")
    button(g, ui, "next", W - BUTTON_W, "▶")
    g.setColor(g.theme.muted)
    g.setFont(14)
    local label = selected and (selected .. " / " .. count) or ("– / " .. count)
    g.printf(label, BUTTON_W, (H - 18) / 2, W - BUTTON_W * 2, "center")
  end,
  click = function(id, args)
    local count = math.max(1, math.floor(tonumber(args.count) or 4))
    local selected = tonumber(args.selected) or 0
    if id == "next" then
      selected = selected % count + 1
    elseif id == "prev" then
      selected = (selected - 2) % count + 1
    else
      return nil
    end
    return {
      args = { selected = selected },
      actions = { { "set", block = args.target or "chart", args = { selected = selected } } },
    }
  end,
}
```

Both blocks change in one edit, so one undo puts both back.
