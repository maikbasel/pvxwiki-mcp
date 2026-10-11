---
paths:
  - "CLAUDE.md"
  - ".claude/rules/**"
  - "README.md"
  - "docs/**"
---

# Documentation

- `CLAUDE.md` stays under 200 lines and holds only what Claude cannot infer from the code. Detail goes
  into a rule file. Repeated multi-step procedures go into a skill in `.claude/skills/`.
- One topic per rule file. Add `paths:` frontmatter, except for rules whose violation is irreversible
  or unsafe. Those must load every session.
- Promote a pattern into a rule only after it has been used twice.
- When a decision changes, update the rule and note why. Never change docs to match broken code.
- `docs/handover.md` is the original brief and is not edited.
