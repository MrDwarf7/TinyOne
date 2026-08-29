# TinyOne language suite

These `.to` fixtures are an integration-test layer for language behavior. They
are run through Rust-only testing hooks, so use:

```sh
cargo test --features testing-hooks
```

The `testing-hooks` feature is intentionally not enabled by default and is not
part of the production API contract.
