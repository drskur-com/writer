# today — inline text (`value`)

Shows today's date where the sheet has `{{today}}`, optionally in another format: `{{today format="%m월 %d일"}}`.

```lua
--[[---
name: today
description: 오늘 날짜를 글로 보인다. 문단 안에 {{today}}로 쓴다.
args:
  format: os.date 형식. 기본값 "%Y-%m-%d".
example:
  format: %Y년 %m월 %d일
---]]

return {
  value = function(args, ctx)
    return os.date(args.format or "%Y-%m-%d")
  end,
}
```
