# TinyOne Documentation Change System

TinyOne uses three public document types to handle change requests, community proposals, and implementation notices without forcing every discussion through the same process.

The goal is to make TinyOne changes visible, structured, debatable, and traceable from initial request through implementation.

## Document Types

TinyOne uses the following document types:

| Type | Name                          | Purpose                                                                      |
| ---- | ----------------------------- | ---------------------------------------------------------------------------- |
| TOR  | TinyOne Request               | A lightweight request for a change, fix, clarification, or improvement.      |
| TOP  | TinyOne Proposal              | A structured community proposal for significant TinyOne changes.             |
| TOIN | TinyOne Implementation Notice | A pre-release or release-facing notice explaining what is being implemented. |

In short:

```text
TOR  = Please consider this.
TOP  = Here is the proposed design.
TOIN = Here is what is being implemented or released.
```

## Core Principle

TinyOne changes should not happen through vague discussion alone.

Each meaningful change should have a written trail that explains:

* what was requested
* why it matters
* what design was considered
* what examples show the behavior
* what tradeoffs exist
* what decision was made
* what is being implemented

This keeps the language, tools, documentation, and ecosystem understandable over time.

## TOR — TinyOne Request

A **TinyOne Request**, or **TOR**, is a lightweight request submitted by a user, developer, contributor, or community member.

A TOR is used when someone wants TinyOne maintainers to consider a change, fix, clarification, or improvement.

TORs are intentionally smaller than full proposals. A user should not need to write a complete language design document to report a valid need.

### Use a TOR for

* feature requests
* confusing behavior
* missing documentation
* unclear compiler diagnostics
* standard library gaps
* tooling improvements
* ecosystem requests
* requests for clarification

### TOR examples

```text
TOR-0001: Improve error message when an import cannot be found
TOR-0002: Add examples for module visibility rules
TOR-0003: Support comments in TinyOne package configuration
TOR-0004: Preserve blank lines in formatter output
```

### TOR lifecycle

```text
Open
Needs Info
Accepted
Rejected
Converted to TOP
Duplicate
Closed
```

### TOR template

```md
# TOR-0000: Request Title

- Status: Open
- Author:
- Created:
- Area: Language | Compiler | Tooling | Documentation | Standard Library | Ecosystem
- Related:

## Request

What are you asking TinyOne developers to change?

## Problem

What problem are you experiencing?

## Example

Show the current issue or desired behavior.

## Expected Benefit

Who benefits, and how?

## Notes

Extra context, links, prior discussion, or constraints.
```

## TOP — TinyOne Proposal

A **TinyOne Proposal**, or **TOP**, is a structured design document for significant TinyOne changes.

A TOP is used when a change requires deeper design discussion, examples, compatibility analysis, tradeoff evaluation, and community review.

TOPs are community-discussed and may be community-authored, but final acceptance should remain maintainer-led unless TinyOne later adopts a formal governance body.

### Use a TOP for

* language syntax changes
* semantic changes
* type system changes
* standard library design
* major compiler behavior
* major tooling behavior
* package or ecosystem conventions
* documentation policy
* governance changes

### TOP examples

```text
TOP-0001: Add immutable bindings
TOP-0002: Define the TinyOne package manifest format
TOP-0003: Standardize compiler diagnostic structure
TOP-0004: Add pattern matching syntax
```

### TOP lifecycle

```text
Draft
Open for Comment
Under Review
Accepted
Rejected
Withdrawn
Deferred
Implemented
Superseded
```

### TOP template

````md
# TOP-0000: Proposal Title

- Status: Draft
- Author(s):
- Created:
- Type: Language | Compiler | Tooling | Standard Library | Governance | Documentation | Ecosystem | Informational
- Related TORs:
- Related TOPs:
- Requires:
- Supersedes:

## Summary

A short explanation of the change.

## Motivation

Why is this needed?
What problem does it solve?
Who is affected by the current behavior?

## Proposed Design

Describe the exact change.

## Examples

Show before and after examples.

### Current Behavior

```tinyone
// current behavior
````

### Proposed Behavior

```tinyone
// proposed behavior
```

## Detailed Semantics

Explain syntax, semantics, edge cases, rules, and constraints.

## Compatibility

Does this break existing code?
Does it require migration?
Can it be introduced gradually?

## Migration

Explain how users should move from the old behavior to the new behavior.

## Alternatives Considered

List other approaches and why they were not chosen.

## Drawbacks

What gets worse?
What complexity does this add?
What risks exist?

## Community Discussion

Link to relevant discussions, objections, agreements, or unresolved points.

## Open Questions

List unresolved design questions.

## Decision

Accepted | Rejected | Deferred | Withdrawn

## Decision Rationale

Explain why this decision was made.

## Implementation Plan

List the implementation steps required.

````

## TOIN — TinyOne Implementation Notice

A **TinyOne Implementation Notice**, or **TOIN**, explains what is coming in TinyOne before or during a release.

A TOIN is not a request and not a proposal. It is a developer-authored notice that explains what has been implemented, what is changing, what users should expect, and how users should migrate if behavior changes.

TOINs are similar in spirit to release notes or a "What’s New" document, but with more structure and stronger links back to TORs and TOPs.

### Use a TOIN for

- upcoming release changes
- accepted TOPs being implemented
- accepted TORs being shipped
- breaking changes
- migration instructions
- deprecated behavior
- compiler or tooling changes
- standard library changes
- implementation status updates

### TOIN examples

```text
TOIN-0001: What is coming in TinyOne 0.2
TOIN-0002: Parser rewrite and diagnostic changes
TOIN-0003: Deprecation of implicit mutable bindings
TOIN-0004: Standard library additions for TinyOne 0.3
````

### TOIN lifecycle

```text
Draft
Scheduled
Published
Updated
Superseded
```

### TOIN template

```md
# TOIN-0000: Notice Title

- Status: Draft
- Release:
- Date:
- Applies To:
- Related TORs:
- Related TOPs:

## Summary

Short explanation of what is changing.

## What Is Changing

Describe the implementation or release change.

## Why It Is Changing

Explain the reason for the change.

## Examples

Show examples of the new behavior.

## Breaking Changes

List any behavior that may break existing code.

## Migration Notes

Explain what users need to do.

## Deprecations

List deprecated behavior and removal timelines.

## Implementation Status

Explain whether the change is complete, partial, experimental, or scheduled.

## Known Limitations

List current limitations or unresolved implementation details.

## Timeline

Describe the expected release or rollout timeline.
```

## Relationship Between TOR, TOP, and TOIN

The three document types should work together, not compete with each other.

```text
TOR → TOP → TOIN
```

A common path looks like this:

```text
1. A user submits a TOR describing a need.
2. The request requires design work.
3. The TOR is converted into or linked to a TOP.
4. The community discusses the TOP.
5. Maintainers accept, reject, defer, or supersede the TOP.
6. Accepted work is implemented.
7. A TOIN explains what is shipping and how users should adapt.
```

Small changes may skip the TOP stage:

```text
TOR → TOIN
```

Example:

```text
TOR-0014: Improve diagnostic for missing imports
→ accepted as a small compiler improvement
→ TOIN-0003 documents improved diagnostics in TinyOne 0.2
```

Major changes should not skip the TOP stage:

```text
TOR → TOP → TOIN
```

Example:

```text
TOR-0021: Add immutable bindings
→ TOP-0007: Immutable and mutable binding semantics
→ TOIN-0005: Binding changes in TinyOne 0.4
```

## Decision Model

TinyOne should prefer open community discussion with clear maintainer responsibility.

Recommended model:

```text
Community-authored.
Community-reviewed.
Maintainer-assisted.
Maintainer-decided.
Publicly documented.
```

A proposal should not be accepted only because it is popular. Language design must preserve coherence, simplicity, and long-term maintainability.

Maintainers should evaluate proposals based on:

* correctness
* consistency with TinyOne’s design goals
* implementation complexity
* compatibility
* migration cost
* ecosystem impact
* clarity of semantics
* long-term maintainability

Every accepted or rejected TOP should include a written rationale.

## Comment Types

Community discussion should be structured where possible.

Recommended comment types:

```text
Support
Concern
Alternative
Clarification
Implementation Note
Blocking Objection
Editorial
```

Example:

````md
Comment Type: Concern

This proposal may create ambiguity with existing function-call syntax.

Example:

```tinyone
foo bar
````

````

This keeps discussion more useful than unstructured agreement or disagreement.

## Required Rules

TinyOne change documents should follow these rules:

1. No TOP without examples.
2. No accepted TOP without documented drawbacks.
3. No rejected TOP without written rationale.
4. No breaking change without migration notes.
5. No major language change without compatibility analysis.
6. No proposal may be accepted if core semantics are ambiguous.
7. One proposal should solve one coherent problem.
8. TORs should stay lightweight.
9. TOINs should be practical and user-facing.
10. Accepted work should link back to the TORs and TOPs that motivated it.

## Suggested Repository Layout

TinyOne can keep these documents in the main repository while the language is young.

```text
tinyone/
  docs/
    changes/
      README.md
      tor-template.md
      top-template.md
      toin-template.md
      tors/
        tor-0001-improve-missing-import-error.md
      tops/
        top-0001-immutable-bindings.md
      toins/
        toin-0001-whats-coming-in-0.2.md
````

If the volume grows, TinyOne can later move these into a dedicated repository:

```text
tinyone-changes/
  README.md
  templates/
  tors/
  tops/
  toins/
  accepted/
  rejected/
  superseded/
```

## Summary

TinyOne uses three related document types:

* **TOR** captures needs.
* **TOP** designs changes.
* **TOIN** announces implementation.

This gives TinyOne a public, structured, and maintainable process for language, tooling, documentation, standard library, ecosystem, and governance changes.
