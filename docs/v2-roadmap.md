# TinyOne v2 Roadmap

TinyOne is now in the v2 language generation. V2 expands the language core
while preserving the existing VM, JIT, Ralloc-backed heap, bytecode verifier,
and VM/JIT parity requirements.

## V2 language commitments

### First-class functions and closures

Function declarations are values: referencing a top-level function produces a
first-class function value, and a function-valued variable can be called with
ordinary call syntax.

Closures are heap values containing a function identity and captured values.
`closure_new(function_name, captures)` constructs one, and calling the result
prepends its captures to the explicit arguments. The bytecode surface is
`PUSH_FUNCTION` plus `CALL_VALUE`; both the interpreter and JIT implement the
same dispatch and call-depth checks.

V2 follow-up work includes source-level closure literals, capture inference,
and richer closure diagnostics. The runtime representation and indirect call
ABI are established now so those features can be added without another value
model migration.

### Generic functions

V2 accepts generic function declarations:

```tinyone
fn identity<T>(value: T) -> T { return value }
```

Generic parameters are recorded in `Function` metadata and bytecode artifacts.
They are erased during execution because TinyLang values remain dynamically
typed; one function body serves every runtime instantiation. V2 follow-up
work includes generic constraints, explicit instantiation syntax, and static
type checking where those constraints provide value.

## Implementation tracks

- Add source-level closure literals and lexical capture analysis.
- Define generic constraints and type-argument diagnostics without weakening
  runtime safety.
- Keep `Program` fingerprints, artifacts, verifier rules, VM, JIT, and FFI
  representations synchronized as the language grows.
- Maintain parity tests for direct calls, function values, captured calls,
  generic declarations, artifacts, and both execution backends.
- Preserve the frozen v1 ABI policy as v2 evolves; entering v2 does not silently
  make the C ABI unstable for existing v1 consumers.

## V2 status

The first v2 slice is implemented: first-class top-level function values,
callable heap closures, erased generic declarations, generic metadata in
artifacts, verifier support, and VM/JIT parity coverage.

