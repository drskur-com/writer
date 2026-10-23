# Actions and talking to other blocks (`click` → `actions`)

Read `interactive.md` first. Besides changing its own block's arguments, `click` can return **actions** that the app runs: change another block, run another extension's command, put text into the sheet, open a sheet, or open an app screen. The extension never touches the app itself; it returns a list and the app runs it. Example: `examples/chart-remote.md`.

## Returning actions

```lua
click = function(id, args)
  return {
    args = { page = 2 },                                         -- this block, as before
    actions = {
      { "set", block = "chart", args = { selected = 3 } },       -- another block
      { "editor.insert", text = "…" },
    },
  }
end
```

Returning a plain table (`{ page = 2 }`) still means "change these arguments of this block". With `args` or `actions` keys, `args` is this block's change and `actions` is a list run in order. At most 16 actions per click.

| Action | Fields | What the app does | Asks the user first |
|---|---|---|---|
| `set` | `block`, `args` | Changes the arguments of every block in the same sheet whose `id:` is `block`. Together with this block's own `args` it is one edit (one undo). | No |
| `command` | `extension`, `index` (from 1) or `title` | Runs that extension's Extensions-menu command on the current selection. | Yes, once |
| `editor.insert` | `text` | Inserts the text at the cursor. | Yes, once |
| `editor.replaceSelection` | `text` | Replaces the selection with the text (inserts at the cursor if nothing is selected). | Yes, once |
| `open` | `sheet` (path from the library root, for example `Notes/Ideas/plan.md`) | Opens that sheet. | No |
| `menu` | `action`: `"preview"` or `"compile"` | Toggles the book preview, or opens the compile (export) dialog. | No |

## Declaring actions

List every action the extension may return on one header line, or `click` fails with an error:

```lua
--[[---
description: …
actions: set, editor.insert
---]]
```

The first time an extension returns an action marked "Asks the user first", the app shows the user what it wants to do; once allowed, that extension does it without asking again. So return such actions only when the user clicked something that clearly means it (an "Insert date" button), never on every click.

## Talking to another block

Blocks run separately and cannot read each other. To make block B react to a button in block A:

1. Give B a name with the reserved argument `id:` (for example `id: chart`). Users can set it in the block's edit dialog; tell them to.
2. In A's `click`, return `{ "set", block = "chart", args = { … } }` with B's arguments as they should become. B redraws with them.
3. A cannot see B's current arguments. If A needs to know the state (for example to step through pages), keep it in A's own arguments too and change both in the same click.

Let the target block's id be an argument of A (for example `target`, default `"chart"`) so one sheet can hold several pairs.

## Testing

`run_extension` with `click` lists the actions `click` returned (it does not run them) and draws this block with its changed `args`. Check that each action has the right fields. To see the target block's look, call `run_extension` on the target extension with the arguments the action sets.
