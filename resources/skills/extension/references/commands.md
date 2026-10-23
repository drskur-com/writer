# Command extensions (`commands`)

Read this before writing or changing a command. Example: `examples/text-tools.md`.

## `run(ctx)`

`run(ctx)` gets:

| Field | Meaning |
|---|---|
| `ctx.selection` | Selected text (empty when nothing is selected). |
| `ctx.text` | The whole sheet as the editor holds it: one line per paragraph. |
| `ctx.sheet` | The sheet's path from the library root. |

Return a string (or number) to replace the selection, or insert it at the cursor when nothing is selected. Return `nil` to change nothing. Calling `error("message")` shows the message to the user. Limit: 2 s.

## Rules

- **Commands.** Return a string to replace the selection (or insert at the cursor when nothing is selected), or `nil` to change nothing. Keep the selection's text intact unless the command is meant to change it. For anything that counts or cuts characters in Korean or other non-ASCII text, use `utf8.len`, `utf8.codes`, `utf8.offset`, and `utf8.char`; `#s`, `s:sub`, `s:reverse`, and `s:upper` work on bytes and break Korean.

## Testing

Run each command with `run_extension`, with a sample selection that includes Korean or other non-ASCII text, and with an empty selection.
