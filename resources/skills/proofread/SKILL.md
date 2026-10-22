---
name: proofread
description: Check spelling, spacing, and punctuation. Use when the user asks to proofread, fix typos, or check spelling or spacing. Does not change style or content.
---

# Proofread

Find errors of form only. Do not improve sentences (that is the revise skill).

## Language rules
- Apply the conventions of the manuscript's language.
- If the manuscript is in Korean, read `references/ko.md` first and follow it.
- For other languages, follow that language's standard spelling, spacing, and punctuation rules.

## Check
- Spelling and commonly confused words.
- Spacing and word separation.
- Punctuation: paired quotation marks, ellipses, doubled commas or periods.
- Spelling of loanwords and proper nouns, consistently across the text.

## Do not
- Do not correct nonstandard words, dialect, or colloquialisms that are clearly intentional, such as in dialogue or a character's voice. If you cannot tell, ask instead of correcting.
- Do not touch word order, word choice, or sentence length.
- Do not treat Markdown syntax (`#`, `*`, `[^1]`, `{.unnumbered}`, and so on) as errors.

## How to answer
- Answer in the user's language.
- Propose all fixes for the sheet in a single edit_file call; the user reviews them as a diff and applies or declines them. Use `all` for an error that repeats.
- In your reply, give one short line per fix with the reason, in the order the errors appear (for example "열엇다 → 열었다: typo"). Do not repeat whole sentences.
- If you are unsure whether something is intentional, leave it out of the edit and ask about it in your reply.
- If there are no errors, say so in one line and propose nothing.
- If no tools are available, show each fix in the original/revised/reason format instead.
