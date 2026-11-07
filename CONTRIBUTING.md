# Contributing

## One-time setup

Install the hook runner and linters (versions pinned here are the ones CI uses):

```sh
brew install lefthook typos-cli cargo-deny cargo-shear
# or: cargo binstall lefthook typos-cli cargo-deny cargo-shear
```

| Tool | Version |
| --- | --- |
| lefthook | 2.2.2 |
| typos-cli | 1.52.0 |
| cargo-deny | 0.20.2 |
| cargo-shear | 1.14.0 |

Then install the git hooks once per clone:

```sh
lefthook install
```

If `core.hooksPath` is set globally, lefthook refuses to install. Either set it locally for this repo (`git config --local core.hooksPath "$(git rev-parse --git-common-dir)/hooks"`) and run `lefthook install --force`, or unset the global value.

## What runs

- pre-commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `typos`, `cargo shear`, `cargo deny check licenses bans sources`.
- pre-push: `cargo deny check advisories` (needs network) and `cargo test --workspace`.
- CI runs `lefthook run pre-commit --all-files` in the `lint` job, so the hook and CI use the same commands.

Lint levels live in `[workspace.lints]` in `Cargo.toml`; every crate inherits them with `lints.workspace = true`. An `allow` needs a one-line reason.
