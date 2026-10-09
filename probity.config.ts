// TDD guardrail for coding agents: https://github.com/nizos/probity
import { defineConfig, enforceTdd } from '@nizos/probity'

const rustLayout = `### Project layout (Rust)

- Unit tests live in the same file as the code, inside \`#[cfg(test)] mod tests\`. A write to a
  file in a crate's src/ can therefore be a test change, an implementation change, or both. Judge each part.
- Acceptance tests live in the bin crate's tests/ and call MCP tools through the public API.
  Pure library crates (the parser) are tested through their public functions in their own tests/.
- Outside-in: while an acceptance test is red, inner red-green-refactor cycles with unit tests are
  the expected way to make it pass. A red acceptance test alone does not justify a large
  implementation step; each inner step still needs its own failing unit test.
- A failing test means \`cargo test\` output showing an assertion failure or panic in that test. A
  compile error from a test that references code that does not exist yet also counts as red.`

export default defineConfig({
  rules: [
    {
      files: ['crates/*/src/**/*.rs', 'crates/*/tests/**/*.rs'],
      rules: [enforceTdd({ instructions: (defaults) => `${defaults}\n\n${rustLayout}` })],
    },
  ],
})
