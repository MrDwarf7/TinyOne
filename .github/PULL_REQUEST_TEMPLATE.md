# Summary

> Please delete options that are not relevant.

- Fixes #`<issue>`
  or
- Closes #`<issue>`

> [!IMPORTANT]
>
> - Please include a summary of the change and which issue it fixes/closes.
>   Please also include relevant motivation and context.

... Your summary here ...

## Description

> - Please provide a reasonable length description of what you did in the PR.
>   This helps reviewers get a clear understanding quickly of what's been done.

... Your description here ...

## Type of change

> Please delete options that are not relevant.

- [ ] - Bug fix (non-breaking change which fixes an issue)
- [ ] - New feature (non-breaking change which adds functionality)
- [ ] - Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] - Documentation update

## Checklist

<details>
  <summary>
    Checklist tick-boxes
  </summary>
<p>

1. Conformity

- [ ] - My code follows the style guidelines of this project
- [ ] - Code is formatted with `cargo fmt --all`
- [ ] - TOML is formatted with `taplo format`
- [ ] - No clippy warnings (run `cargo make ci-clippy` or `cargo clippy --workspace --all-targets -- -D warnings`)

2. Best-Effort

- [ ] - My changes generate no new warnings
- [ ] - I have performed a self-review of my own code
- [ ] - I have commented my code, particularly in hard-to-understand areas

3. Documentation

- [ ] - Any documentation that relates to this PR has been updated
    or will be updated in a PR (Link here: )(if applicable)
- [ ] - `docs/INDEX.md` / `docs/CONTRIBUTING.md` / `docs/EXAMPLES.md` references updated if files moved (uppercase policy)

4. Tests & ABI

- [ ] - I have added tests that prove my fix is effective or that my feature works
- [ ] - New and existing unit tests pass locally (`cargo make ci-test` / `cargo test --workspace`)
- [ ] - Language suite passes (`cargo test --features testing-hooks --test language_suite`)
- [ ] - `tinylang.h` / `cbindgen` output updated if FFI changed (`cargo make ffi-consumer-compile` / `uv run python tools/abi_manifest.py check`)
- [ ] - ABI drift check passes (`uv run --no-project --python 3.12 python tools/abi_manifest.py check`)

5. Dependencies

- [ ] - Any dependent changes have been merged and published in downstream modules
- [ ] - Version pins updated in native files (`rust-toolchain.toml`, `.python-version`, `setup-gcc` inputs) + `docs/CI_CD.md` if changed

</p>
</details>
