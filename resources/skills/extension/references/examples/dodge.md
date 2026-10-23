# dodge — a small game (`play`)

Move with ←/→ and dodge the falling blocks; the score counts the seconds survived. Space restarts. The block shows a title card with the app's ▶ button. Block:

````
```ext dodge
speed: 1
```
````

```lua
--[[---
name: dodge
description: 떨어지는 상자를 피하는 작은 게임. ▶를 누르면 창에서 시작한다. ←/→로 움직이고 스페이스로 다시 시작한다.
args:
  speed: 상자가 떨어지는 빠르기. 기본값 1.
example:
  speed: 1
---]]

local W, H = 360, 240
local PLAYER_W, PLAYER_H = 36, 12

local x, boxes, timer, score, over, left, right, speed

local function reset()
  x, boxes, timer, score, over = W / 2, {}, 0, 0, false
end

return {
  size = { W, 120 },
  export = false,
  draw = function(g, t, args)
    g.setColor(g.theme.foreground)
    g.setFont(20)
    g.printf("상자 피하기", 0, 30, W, "center")
    g.setColor(g.theme.muted)
    g.setFont(13)
    g.printf("▶를 눌러 시작 · ←/→ 이동 · 스페이스 다시 시작", 0, 68, W, "center")
  end,
  play = {
    size = { W, H },
    load = function(args)
      speed = tonumber(args.speed) or 1
      left, right = false, false
      math.randomseed(os.time())
      reset()
    end,
    keypressed = function(key)
      if key == "left" then left = true end
      if key == "right" then right = true end
      if key == "space" and over then reset() end
    end,
    keyreleased = function(key)
      if key == "left" then left = false end
      if key == "right" then right = false end
    end,
    update = function(dt)
      if over then return end
      score = score + dt
      local move = (right and 1 or 0) - (left and 1 or 0)
      x = math.max(PLAYER_W / 2, math.min(W - PLAYER_W / 2, x + move * 220 * dt))
      timer = timer - dt
      if timer <= 0 then
        timer = 0.6 / speed
        table.insert(boxes, { x = math.random(10, W - 10), y = -10, size = math.random(10, 22) })
      end
      for i = #boxes, 1, -1 do
        local box = boxes[i]
        box.y = box.y + (90 + score * 6) * speed * dt
        local hitX = math.abs(box.x - x) < (box.size + PLAYER_W) / 2
        local hitY = box.y + box.size / 2 > H - 20 - PLAYER_H and box.y - box.size / 2 < H - 20
        if hitX and hitY then over = true end
        if box.y > H + 20 then table.remove(boxes, i) end
      end
    end,
    draw = function(g)
      g.setColor(g.theme.accent)
      g.rectangle("fill", x - PLAYER_W / 2, H - 20 - PLAYER_H, PLAYER_W, PLAYER_H, 3)
      g.setColor(g.theme.foreground)
      for _, box in ipairs(boxes) do
        g.rectangle("fill", box.x - box.size / 2, box.y - box.size / 2, box.size, box.size, 2)
      end
      g.setColor(g.theme.muted)
      g.setFont(13)
      g.print(string.format("%.1f초", score), 8, 8)
      if over then
        g.setColor(g.theme.foreground)
        g.setFont(18)
        g.printf("끝! 스페이스로 다시", 0, H / 2 - 12, W, "center")
      end
    end,
  },
}
```

All game state is in the file's `local` variables and `reset()`/`load` set every one of them. Held keys are tracked with `left`/`right` flags so movement is smooth in `update`.
