# Decision records

A decision record captures one design decision: what was decided, why, and what follows from it. Write one when a change settles a question the brief leaves open (`docs/brief.md`, §13), or makes a choice a later contributor would otherwise have to reverse-engineer from the code.

## Files

Each record is `docs/decisions/NNNN-<slug>.md`: a four-digit number, one higher than the highest existing record, and a short lower-case slug with hyphens, for example `0001-playback-engine.md`. Numbers are never reused, and a record is never renumbered.

## Format

```markdown
# NNNN. <Title>

## Status

Proposed

## Context

The situation and the forces at play: the question, the constraints, the options considered and what was learned about each.

## Decision

What was decided, stated in full sentences.

## Consequences

What becomes easier or harder, what must now be done, and what is ruled out.
```

## Status

- **Proposed**: the record is under review in a pull request. Every record starts here.
- **Accepted**: the responsible engineer accepts a record by merging it. Once they approve, the pull request's last commit changes the status to Accepted, so every record on `main` is Accepted or Superseded. Nothing that depends on a decision merges before its record.
- **Superseded**: a later record replaces it. Change the status to `Superseded by NNNN`, linking the new record, and leave the rest of the old record as it was.

A decision record is always a human merge: only the responsible engineer accepts a decision.
