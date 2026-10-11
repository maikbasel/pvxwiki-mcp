# Data fidelity

Always loaded. The product is trustworthy build data. A guessed field or a flattened nuance is
worse than a missing one, because the agent cannot tell.

## Required

- Capture every section and infobox a page has, not a fixed list. A section the parser does not
  recognise is kept as named raw text and stays searchable. It is never dropped.
- Keep what the markup marks as structured data: the main skill bar, its `Optional` slots, and
  each named variant bar. Recommendations written as prose (equipment, optional skills listed in
  text) stay in the section text for the agent to read; the parser does not extract them.
- Each variant keeps its own skill bar, attributes and notes. Variants are not merged into the main
  build.
- The parser is a pure function from wikitext to a typed build. It does no I/O. It emits
  `tracing` events only (`logging.md`); its output never depends on logging.

## Forbidden

- Inventing or defaulting a value the page does not state. Missing is `None`, not a guess.
- Discarding a whole page because one section fails to parse. Parse the rest and record the failure.
- Replacing a stored page with a worse parse. If a new revision fails entirely, keep the last good
  copy and log a warning with the page title and revision ID.

## Debugging a parser bug

Put the raw wikitext of the failing section next to the parser output before changing the parser.
Then add that page as a fixture test (`testing.md`).
