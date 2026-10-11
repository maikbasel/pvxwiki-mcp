---
paths:
  - "crates/*/src/**/*.rs"
  - "crates/*/tests/**/*.rs"
  - "crates/*/tests/fixtures/**"
---

# Testing

## Workflow: outside-in

1. **Outer loop.** A feature starts with one failing acceptance test in the bin crate's `tests/`
   that calls an MCP tool and asserts on the response. It stays red until the feature works end to end.
2. **Inner loop.** Red-green-refactor with unit tests in `#[cfg(test)] mod tests` next to the code,
   until the acceptance test passes.
3. A bug fix starts with a test that reproduces it and fails for the expected reason. Use the
   outermost level that can show the bug.

An acceptance test changes only when the requirement changes, never to make it pass.

## Ports and adapters

The core depends on ports (traits); adapters implement them for the wiki and the database.

- **Core:** tested through its ports with hand-written in-memory fakes. Assert on outcomes (what is
  stored, what is returned), never on which port methods were called. No mocking library.
- **Adapters:** tested against the real system or the closest stand-in: the real storage engine in a
  temp directory (Testcontainers if the engine is a server), and the wiki faked at the HTTP level with
  `wiremock`, including 429, `maxlag` and 403 responses.
- **Contract tests:** each port has one test suite, written as a generic function over the trait, that
  runs against both the fake and the real adapter. A fake without a passing contract suite is not used.
- **Acceptance tests:** call MCP tools with real adapters (temp-dir database, `wiremock` wiki) and
  assert on the response shape, including attribution.
- **Pure logic** (parsing, search ranking, revision diffing) needs no doubles. The parser is tested
  against real recorded wikitext in `crates/parser/tests/fixtures/pages/`: one fixture per page
  shape (single build, team build, build with variants, page with an unknown section), file name
  including the page title and revision ID it was recorded from.

A port trait with a production adapter and a test fake has two implementations; it does not break the
"no trait with one implementation" rule in `production-grade.md`.

## Forbidden

- Any test that reaches the live wiki. Recording a new fixture is a manual, one-off step.
- Asserting on log output or exact error prose. Assert on stable error kinds and response fields.
