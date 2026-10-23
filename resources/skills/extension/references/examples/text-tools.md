# text-tools — commands on the selection

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
