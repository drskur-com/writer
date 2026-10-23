# Inline extensions (`{{name key=value}}`)

An extension can also appear inside a paragraph or heading: `오늘은 {{today}}입니다.` The editor shows the call as one object; when the caret touches it, the source `{{today}}` appears for editing, with the result shown faded right after it. Copying copies the source, and the book preview and exports show its text. Users can also edit its arguments in a dialog by double-clicking it. Example: `examples/today.md`.

## Writing a call

`{{name key=value key="value with spaces"}}`: the extension's file name (starting with a letter), then arguments as `key=value`. Quote values that contain spaces, `"` (as `\"`), `{`, or `}`. Calls inside inline code (`` `{{x}}` ``) stay plain text. The reserved `export=false` leaves it out of exports, as for blocks. `scale=2` makes the object twice the line height (and its line's rows taller to fit; `0.5` makes it smaller); inside a heading objects grow with the heading. `align` does not apply inline.

## Text: `value(args, ctx)`

```lua
return {
  value = function(args, ctx)
    return os.date(args.format or "%Y-%m-%d")
  end,
}
```

- Return a string (or number): it is shown in place of the call, in the editor as a small chip and in exports as ordinary text in the surrounding style. Only the first line is used. Return `""` to show nothing in exports.
- `ctx.sheet` is the sheet's path from the library root. `args` arrive like block arguments (numbers and booleans converted).
- The editor asks again at most once a minute and whenever the text or the extension changes, so a value may lag by up to a minute. Keep it fast (0.1 s limit) and free of side effects.
- An extension can have `value` together with `draw` or `commands`; with `value`, the inline call shows the text.

## Small pictures: `draw` inline

An extension with `draw` (and no `value`) appears inline as its canvas, scaled to the line height (times `scale`): design it for a small square or a wide strip (for example `size = { 20, 20 }` or `{ 60, 16 }`), with shapes that read at text size. Regions and `click` work inline too (`interactive.md`); a click's `args` are written back into the call. Inline pictures are not exported yet: the book preview and exports leave them out.

## Testing

`run_extension` reports the text `value` returns for the given `args`. Test with and without arguments, and with non-ASCII arguments.
