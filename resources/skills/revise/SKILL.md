---
name: revise
description: Find awkward or hard-to-read sentences and polish them. Use when the user asks to fix awkward sentences, polish, tighten, or make the text easier to read.
---

# Revise

Fix only where readers stumble, while keeping the writer's voice.

## Look for
- Subjects and predicates that do not agree; modifiers whose target is ambiguous.
- Long sentences that tangle too many ideas.
- Words or expressions repeated close together; runs of the same sentence ending.
- Translationese and padding: needless nominalizations, double passives, filler that adds no meaning.

## Keep
- The writer's tone, point of view, tense, and level of formality.
- Rhythm that is clearly deliberate: short punchy fragments, long flowing sentences, repetition used for emphasis.
- The content. Do not add or remove facts, events, or worldbuilding.
- Sentences that do not need fixing. Fewer changes are better.

## How to answer
- Answer in the user's language.
- Choose at most ten places, most impactful first, and propose them in a single edit_file call; the user reviews them as a diff and applies or declines them.
- In your reply, give one line per place with the reason (for example, "the subject and the verb do not agree"). Do not repeat the revised sentences.
- If no range was given, review the whole open sheet.
- If no tools are available, show each change in the original/revised/reason format instead.
