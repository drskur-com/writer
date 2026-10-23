---
name: extension
description: Write, change, or fix a Writer extension — a Lua script in Extensions/ that draws a picture, chart, or animation inside a sheet (a ```ext block) or adds a command to the Extensions menu (for example inserting today's date or transforming the selected text). Use when the user wants something drawn or animated in the editor, a custom text command or shortcut action, or when an extension block or command shows an error.
---

# Writer extensions

An extension is one Lua (Luau) file, `Extensions/<name>.lua`, that returns a table. The app uses two entry points:

- `draw(g, t, args)` draws on a canvas wherever a sheet has a ```` ```ext <name> ```` block. `g` is a canvas with the LÖVE `love.graphics` API. `g.newScene()` adds simple 3D (boxes, spheres, meshes, a camera and a light), drawn as flat-shaded 2D shapes.
- `commands` adds items to the app's **Extensions** menu. A command gets the selected text and returns the text that replaces it.

Before writing code, read `references/api.md` (the complete API: module fields, every `g` function, `ctx`, the sandbox). For a new extension, also read `references/examples.md` and start from the closest example.

Answer in the user's language. Write code, comments, and file names in English unless the user asks otherwise. The header's `description` and `args` lines are shown to the user, so write them in the user's language. Text that the drawing or command shows to readers (labels, inserted text) follows the manuscript's language.

## Workflow

1. **Decide what to build.** A picture or animation in the sheet is `draw`. An action on text is a command. One file can have both. If the request is ambiguous (for example "a date": insert it once, or draw it), pick the reading that fits best and say which one you chose. Ask only when the choice changes the result a lot.
2. **Check what exists.** The `<extensions>` list in your instructions shows the existing extensions with their descriptions. If one already does what the user wants, use it (add its block, step 5) instead of writing a new one. If one is close, `read_file` it and change it with `edit_file`. Name a new extension with short lowercase English words joined by `-` (for example `archer`, `bar-chart`, `insert-date`).
3. **Write the file** with `write_file` at `Extensions/<name>.lua`, in one file. Start it with the header (`references/api.md`, "The header"): `description`, `args` with each argument's meaning and default, and `example` values that show the drawing well. The user picks extensions by this header in Insert → Extension…, so keep it accurate whenever you change the code.
4. **Test it with `run_extension`. Never skip this step.**
   - Draw: call at `t = 0` with the arguments the block will use. If animated, also call at a few later times (for example 0.7 and 1.9) and at the end of a loop. Also call once without arguments to check the defaults.
   - The result reports the shape count and the drawing bounds. Fix it if anything is drawn outside the canvas, if nothing is drawn, or if the shape count is in the thousands.
   - Commands: run each one with a sample selection that includes Korean or other non-ASCII text, and with an empty selection.
   - Test with the header's `example` values too: that is what the user sees in the Insert dialog's preview.
   - If there is an error, fix the file with `edit_file` and test again. The user sees the error in the sheet otherwise. Also fix any `Note:` about the header.
5. **Place the block** if the user wants the drawing in a sheet. Add the ```` ```ext ```` block with `edit_file`, with a blank line before and after it, where the user asked (by default at the end of the open sheet). Put arguments one per line as `key: value`. If the user did not ask for it in a sheet, tell them they can insert it with Insert → Extension….
6. **Reply briefly**: what the extension does, how to use it (the block, **Insert → Extension…**, or the menu item **Extensions → title**), and how to change it. Do not paste the whole script; it is in the file.

## Design rules

- **Theme colors.** Use `g.theme.foreground`, `.muted`, `.accent`, `.border`, and `.background` for lines, text, and backgrounds so the drawing works in light and dark themes. Use fixed colors only for things that have their own color (a red target, a blue sky).
- **Size.** Set `size` to the space the drawing needs, usually 300–600 wide and 100–300 high. The app shrinks a canvas that is wider than the page and never enlarges it. When the space depends on the arguments (for example more bars), make `size` a `function(args)` returning `{ width, height }`. To make a drawing bigger or smaller, the user writes `scale: 1.5` in the block; the app handles it for every extension, so never define a `scale` argument. Lay things out from `g.getWidth()` and `g.getHeight()` rather than repeating numbers.
- **Arguments.** Every argument needs a default (`local power = args.power or 1`). Numbers, `true`, and `false` arrive as numbers and booleans; everything else as strings. Split lists with `string.split(args.values or "", ",")` and convert each item with `tonumber`.
- **Animation.** Compute everything from `t`, for example `local phase = (t % period) / period`. `draw` keeps no state between calls and starts each call from an empty canvas. Make the frame at `t = 0` meaningful on its own: exports will use it (PDF, EPUB, and DOCX export of extension blocks is not built yet; they currently come out as code blocks). For a loop, set `period` to its length in seconds (at most 10): the app caches one loop's frames and stops calling `draw`. Leave `animated` off for still pictures; an animation redraws the window 30 times a second while visible.
- **Randomness.** `math.random` restarts from the same seed on every draw, so random stars or noise stay still and look the same every time. To vary them over time, derive the values from `t`.
- **Text.** `g.print` draws one line per `\n` and does not wrap. Use `g.printf(text, x, y, width, "center")` to center text. Keep labels short; text does not rotate.
- **Commands.** Return a string to replace the selection (or insert at the cursor when nothing is selected), or `nil` to change nothing. Keep the selection's text intact unless the command is meant to change it. For anything that counts or cuts characters in Korean or other non-ASCII text, use `utf8.len`, `utf8.codes`, `utf8.offset`, and `utf8.char`; `#s`, `s:sub`, `s:reverse`, and `s:upper` work on bytes and break Korean.
- **Limits.** Each draw call has 0.1 s (an animation frame at most 2,000 shapes), loading the file 0.1 s, and each command 2 s; the script has no files, network, `io`, `os.execute`, or `require`. If the user asks for something the API cannot do (images, clicking, reading other files, network, realistic 3D with textures, shadows, or smooth shading), say so and offer the closest thing it can do.

## Fixing an extension

When the user reports a problem or a block shows an error ("확장 <name>: ..."), `read_file` the extension, reproduce the problem with `run_extension` using the block's arguments from the sheet, fix it, and test again. Error messages name the function and argument (for example `g.circle: argument #3 must be a number`) or the missing function.
