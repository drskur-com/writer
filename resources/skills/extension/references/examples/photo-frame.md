# photo-frame — the user's photo in a frame

Block:

````
```ext photo-frame
src: Notes/Attachments/beach.jpg
caption: 여름, 바닷가에서
```
````

```lua
--[[---
name: photo-frame
description: 사진을 폴라로이드 테두리로 감싸 살짝 기울이고 아래에 캡션을 단다.
args:
  src: 사진 경로(라이브러리 기준, 예 Notes/Attachments/사진.jpg)
  caption: 사진 아래 글
  tilt: 기울기(도). 기본값 -4
  style: polaroid(기본), round, circle
example:
  src: Notes/Attachments/beach.jpg
  caption: 여름, 바닷가에서
---]]
local W, H = 420, 360

local function cover(img, x, y, w, h)
  -- 칸을 꽉 채우고 넘치는 쪽은 자른다.
  local iw, ih = img:getDimensions()
  local s = math.max(w / iw, h / ih)
  g.draw(img, x + (w - iw * s) / 2, y + (h - ih * s) / 2, 0, s)
end

return {
  size = { W, H },
  draw = function(g, t, args)
    if not args.src then
      g.setColor(g.theme.muted)
      g.printf("src: 사진 경로를 적어 주세요", 0, H / 2 - 8, W, "center")
      return
    end
    local img = g.newImage(args.src)
    local style = args.style or "polaroid"
    if style == "circle" then
      local r = 140
      g.setShadow({ 0, 0, 0, 0.35 }, 10, 0, 6)
      g.setColor(1, 1, 1)
      g.circle("fill", W / 2, H / 2, r + 8)
      g.setShadow()
      g.setClip(g.newPath():circle(W / 2, H / 2, r))
      cover(img, W / 2 - r, H / 2 - r, 2 * r, 2 * r)
      g.setClip()
      return
    end
    local fw, fh = 300, 300
    g.push()
    g.translate(W / 2, H / 2)
    g.rotate(math.rad(tonumber(args.tilt) or -4))
    g.translate(-fw / 2, -fh / 2)
    g.setShadow({ 0, 0, 0, 0.35 }, 10, 4, 8)
    g.setColor(1, 1, 1)
    if style == "round" then
      g.rectangle("fill", 0, 0, fw, fh, 18)
    else
      g.rectangle("fill", 0, 0, fw, fh)
    end
    g.setShadow()
    local pad = 14
    local photoH = style == "round" and fh - pad * 2 or fh - pad - 56
    local clip = g.newPath():rect(pad, pad, fw - pad * 2, photoH, style == "round" and 10 or 0)
    g.setClip(clip)
    cover(img, pad, pad, fw - pad * 2, photoH)
    g.setClip()
    if style ~= "round" and args.caption then
      g.setColor(0.2, 0.2, 0.2)
      g.printf(args.caption, 0, fh - 42, fw, "center")
    end
    g.pop()
  end,
}
```
