# MACH Engineering Contract

## Rules

- Write only necessary code.
- No redundant comments.
- No speculative abstractions.
- No unnecessary wrappers.
- No dead code.
- No unused dependencies.
- Never invent framework APIs.
- Prefer existing MACH primitives.
- Prefer borrowing over cloning.
- Never block async workers.
- Never use unbounded queues.
- Preserve explicit authorization boundaries.

## Workflow

- Run `mach context --compact` first.
- Use MACH CLI for structural changes.
- Use `mach api SYMBOL` when uncertain.
- Do not inspect generated code unless necessary.
- Run `mach check --agent` after changes.
- Run affected tests.
- Fix all proven violations.

## Output

- Keep responses concise.
- Do not narrate implementation.
- Do not generate unsolicited documentation.
