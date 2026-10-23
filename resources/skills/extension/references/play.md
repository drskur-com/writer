# Play: running an extension in its own window (`play`)

For games and other things that need a running state, the keyboard, or dragging, an extension adds a `play` table. The block in the sheet still shows `draw` (a still title picture, with a ▶ button the app adds); pressing ▶ opens a window where `play` runs. Nothing in the sheet changes while playing, and exports show only the block's `draw`. Example: `examples/dodge.md`.

## The `play` table

```lua
local x, score
return {
  draw = function(g, t, args) … end,         -- the block: a still title picture
  play = {
    size = { 360, 240 },                       -- canvas of the window (default: the block's size)
    load = function(args) x, score = 180, 0 end,
    update = function(dt) … end,               -- seconds since the last frame (at most 0.1)
    draw = function(g) … end,                  -- the same canvas API as blocks
    keypressed = function(key) … end,
    keyreleased = function(key) … end,
    mousepressed = function(x, y, button) … end,
    mousereleased = function(x, y, button) … end,
    mousemoved = function(x, y) … end,
  },
}
```

- Only `play.draw` is required. Each frame the app calls the input callbacks for what happened since the last frame, then `update(dt)`, then `draw(g)`, about 60 times a second while the window is active (it pauses when inactive).
- **State lives in Lua.** Unlike block `draw`, `play` keeps its variables between calls: keep the game state in `local` variables of the file (or a table) and reset them in `load(args)`. `args` are the block's arguments. Each window starts a fresh copy of the file.
- `draw(g)` starts from an empty canvas each frame, like block drawing; `g.theme` has the theme colors. At most 2,000 shapes per frame and 0.1 s per call.
- Keys are named like `"left"`, `"right"`, `"up"`, `"down"`, `"space"`, `"enter"`, `"escape"`, `"backspace"`, `"tab"`, and single characters (`"a"`, `"1"`). `keypressed` is not repeated while a key is held: track held keys yourself (set a flag in `keypressed`, clear it in `keyreleased`, move in `update`).
- Mouse positions are canvas coordinates; `button` is 1 (left), 2 (right), 3 (middle).
- `math.random` is not reseeded between frames in play, so it varies; seed it in `load` (for example `math.randomseed(os.time())`) when you want each run to differ.
- An error stops the game and shows the message in the window.
- **Sound.** In play, `sound.play{ … }` plays notes (see below). Blocks are silent: calling `sound` outside `play` is an error.
- A `click` can also open the window: return the action `{ "play", args = { … } }` (declare `play` on the header's `actions:` line, `actions.md`).

## Sound (`sound`)

The app synthesizes notes; there are no sound files. Each call plays at once, over whatever is already playing, and returns an id.

```lua
sound.play{ note = "C5", length = 0.1 }                                   -- one note
sound.play{ notes = { "C4", "E4", "G4", { "C5", 0.4 } }, length = 0.12 }  -- one after another; { note, length } overrides
sound.play{ notes = { "E4", "-", "E4" } }                                -- "-" is a rest
sound.play{ chord = { "C4", "E4", "G4" }, length = 0.6, instrument = "piano" }
sound.play{ note = 36, instrument = "kick" }                              -- MIDI numbers: 60 = C4, 69 = A4
local music = sound.play{ notes = melody, instrument = "pluck", volume = 0.3, loop = true }
sound.stop(music)                                                         -- or sound.stop() for all
```

- Give exactly one of `note`, `notes`, or `chord`. Notes are names (`C4`, `F#3`, `Bb2`; octave 4 has middle C) or MIDI numbers 0–127.
- `length`: seconds per note (default 0.25). `volume`: 0–1 (default 0.5). `loop = true` repeats until `sound.stop`.
- `instrument` (default `square`): `sine`, `square`, `triangle`, `saw` (plain waves, game beeps), `piano`, `organ`, `pluck` (plucked string), `bass`, and drums `kick`, `snare`, `hihat` (their pitch is ignored).
- Limits: 10 seconds and 256 notes per call, 16 sounds at once per window (the oldest stops). Sound pauses with the game when the window is inactive and stops when it closes.
- Keep effects short (0.05–0.3 s) and quiet (volume 0.3–0.5), play them on events (`keypressed`, a hit, a score), not every frame. Loop music at a low volume and stop it on game over.

## Design rules

- Make the block's `draw` a title card that says what it is and how to play (keys), in the manuscript's language, since that is what the sheet and exports show.
- Set `export = false` in the returned table: a game makes no sense on paper.
- Use `dt` for movement (`x = x + speed * dt`) so speed does not depend on the frame rate.
- Keep a clear start, an end (game over, score), and a way to restart (for example `space` or `enter`).

## Testing

`run_extension` draws the block's `draw`. It cannot play; check the play logic by reading it carefully, keep it small, and make sure `load` sets every variable `update` and `draw` read.
