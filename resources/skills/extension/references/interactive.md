# Interactive drawings (`g.region`, `ui`, `click`)

A drawing can react to the mouse in the editor: highlight what the pointer is over, and change the block when the user clicks. Read `draw.md` first; this adds three things to it. Example: `examples/pick-chart.md`.

## Regions

`g.region(id, x, y, w, h)` marks a rectangle that the mouse can hover and click. It draws nothing. `id` is a string you choose (for example `"bar3"`, `"next"`). The rectangle follows the current transform (rotated or scaled, it becomes its bounding rectangle). Declare a region in every frame where it should react, usually right after drawing the thing it belongs to. Where regions overlap, the one declared last wins. At most 256 regions per frame.

Make regions comfortable to hit: at least about 20 × 20 canvas units, and larger than a thin or short shape (cover a bar's whole column, not just the bar).

## `draw(g, t, args, ui)`

`ui` is always a table:

| Field | Meaning |
|---|---|
| `ui.hover` | The `id` of the region under the pointer, or `nil`. |
| `ui.pressed` | The `id` of the region the mouse button is held down on, or `nil`. |

Show hover clearly but quietly (a lighter or accent color, an outline), and pressing as a small change (slightly darker or smaller). The app redraws only when the pointer moves into or out of a region or the button goes down or up, not on every mouse move, and shows a hand cursor over regions. Clicks outside regions go to the editor as usual (they put the caret into the block).

The book preview and exports (PDF, EPUB, DOCX) draw with an empty `ui`, so the picture must look complete without hover: put state that should be printed in the block's arguments, not in `ui`.

## `click(id, args)`

```lua
click = function(id, args)
  if id == "next" then return { page = (args.page or 1) + 1 } end
end
```

Called when the user presses and releases the mouse on the same region. `args` are the block's current arguments. Return a table of arguments to change, or `nil` to change nothing. The app writes each one into the block as a `key: value` line: it replaces the value of an existing line (the last one with that key) or adds a line before the closing fence. Values become text: strings stay as they are, numbers without a fraction are written without `.0`, and booleans as `true`/`false`; they must be one line. An empty string leaves the key with no value (`key:`), which arrives in `args` as `""`.

Because the state lives in the block's text, it is saved with the sheet, undo reverts a click, exports show it, and `draw` sees it in `args` on the next frame. `click` has the same 0.1 s limit as `draw` and keeps no state of its own between calls: everything it needs is in `args`.

Describe arguments that clicks change in the header like any other argument, with their default.

`click` can also return actions for the app: change another block, insert text, run a command, open a sheet (`actions.md`).

## Testing

- Call `run_extension` once without `hover`/`click`: the report lists the regions it declared with their rectangles. Check that each one covers what it should and is big enough.
- Pass `hover` with a region id to see the hover look, and `click` with a region id to call `click` and draw with the arguments it returned (the report says which arguments it set). Click each kind of region at least once, including clicking the same one twice if it toggles.
- Check the picture without `hover` too: that is what exports show.
