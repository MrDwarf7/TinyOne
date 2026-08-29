---
title: Community Forum
---

# TinyLang Community Forum

TinyOne uses three public document types to handle change requests, community proposals, and implementation notices without forcing every discussion through the same process.

The goal is to make TinyOne changes visible, structured, debatable, and traceable from initial request through implementation.

## Document Types

TinyOne uses the following document types:

| Type | Name                          | Purpose                                                                      |
| ---- | ----------------------------- | ---------------------------------------------------------------------------- |
| TLR  | TinyLang Request               | A lightweight request for a change, fix, clarification, or improvement.      |
| TLP  | TinyLang Proposal              | A structured community proposal for significant TinyLang changes.            |
| TLIN | TinyLang Implementation Notice | A pre-release or release-facing notice explaining what is being implemented. |

In short:

```text
TLR  = Please consider this.
TLP  = Here is the proposed design.
TLIN = Here is what is being implemented or released.
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

## TLR — TinyLang Request

A **TinyLang Request**, or **TLR**, is a lightweight request submitted by a user, developer, contributor, or community member.

A TLR is used when someone wants TinyLang maintainers to consider a change, fix, clarification, or improvement.

TLRs are intentionally smaller than full proposals. A user should not need to write a complete language design document to report a valid need.

### Use a TLR for

* feature requests
* confusing behavior
* missing documentation
* unclear compiler diagnostics
* standard library gaps
* tooling improvements
* ecosystem requests
* requests for clarification

### TLR examples

```text
TLR-0001: Improve error message when an import cannot be found
TLR-0002: Add examples for module visibility rules
TLR-0003: Support comments in TinyOne package configuration
TLR-0004: Preserve blank lines in formatter output
```

The number is assigned by the TinyLang system. When submitting a request, do not
choose or type the `TLR-0000` number; leave the identifier blank or use a title
without a number. The system will assign the next available request ID.

### TLR lifecycle

```text
Open
Needs Info
Accepted
Rejected
Converted to TLP
Duplicate
Closed
```

### TLR template

```md
# Request Title

- ID: Assigned by the TinyLang system
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

## TLP — TinyLang Proposal

A **TinyLang Proposal**, or **TLP**, is a structured design document for significant TinyLang changes.

A TLP is used when a change requires deeper design discussion, examples, compatibility analysis, tradeoff evaluation, and community review.

TLPs are community-discussed and may be community-authored, but final acceptance should remain maintainer-led unless TinyOne later adopts a formal governance body.

### Use a TLP for

* language syntax changes
* semantic changes
* type system changes
* standard library design
* major compiler behavior
* major tooling behavior
* package or ecosystem conventions
* documentation policy
* governance changes

### TLP examples

```text
TLP-0001: Add immutable bindings
TLP-0002: Define the TinyOne package manifest format
TLP-0003: Standardize compiler diagnostic structure
TLP-0004: Add pattern matching syntax
```

### TLP lifecycle

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

### TLP template

````md
# TLP-0000: Proposal Title

- Status: Draft
- Author(s):
- Created:
- Type: Language | Compiler | Tooling | Standard Library | Governance | Documentation | Ecosystem | Informational
- Related TLRs:
- Related TLPs:
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

## TLIN — TinyLang Implementation Notice

A **TinyLang Implementation Notice**, or **TLIN**, explains what is coming in TinyLang before or during a release.

A TLIN is not a request and not a proposal. It is a developer-authored notice that explains what has been implemented, what is changing, what users should expect, and how users should migrate if behavior changes.

TLINs are similar in spirit to release notes or a "What’s New" document, but with more structure and stronger links back to TLRs and TLPs.

### Use a TLIN for

- upcoming release changes
- accepted TLPs being implemented
- accepted TLRs being shipped
- breaking changes
- migration instructions
- deprecated behavior
- compiler or tooling changes
- standard library changes
- implementation status updates

### TLIN examples

```text
TLIN-0001: What is coming in TinyOne 0.2
TLIN-0002: Parser rewrite and diagnostic changes
TLIN-0003: Deprecation of implicit mutable bindings
TLIN-0004: Standard library additions for TinyOne 0.3
````

### TLIN lifecycle

```text
Draft
Scheduled
Published
Updated
Superseded
```

### TLIN template

```md
# TLIN-0000: Notice Title

- Status: Draft
- Release:
- Date:
- Applies To:
- Related TLRs:
- Related TLPs:

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

## Relationship Between TLR, TLP, and TLIN

The three document types should work together, not compete with each other.

```text
TLR → TLP → TLIN
```

A common path looks like this:

```text
1. A user submits a TLR describing a need.
2. The request requires design work.
3. The TLR is converted into or linked to a TLP.
4. The community discusses the TLP.
5. Maintainers accept, reject, defer, or supersede the TLP.
6. Accepted work is implemented.
7. A TLIN explains what is shipping and how users should adapt.
```

Small changes may skip the TLP stage:

```text
TLR → TLIN
```

Example:

```text
TLR-0014: Improve diagnostic for missing imports
→ accepted as a small compiler improvement
→ TLIN-0003 documents improved diagnostics in TinyOne 0.2
```

Major changes should not skip the TLP stage:

```text
TLR → TLP → TLIN
```

Example:

```text
TLR-0021: Add immutable bindings
→ TLP-0007: Immutable and mutable binding semantics
→ TLIN-0005: Binding changes in TinyOne 0.4
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

Every accepted or rejected TLP should include a written rationale.

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

1. No TLP without examples.
2. No accepted TLP without documented drawbacks.
3. No rejected TLP without written rationale.
4. No breaking change without migration notes.
5. No major language change without compatibility analysis.
6. No proposal may be accepted if core semantics are ambiguous.
7. One proposal should solve one coherent problem.
8. TLRs should stay lightweight.
9. TLINs should be practical and user-facing.
10. Accepted work should link back to the TLRs and TLPs that motivated it.

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

* **TLR** captures needs.
* **TLP** designs changes.
* **TLIN** announces implementation.

This gives TinyOne a public, structured, and maintainable process for language, tooling, documentation, standard library, ecosystem, and governance changes.
