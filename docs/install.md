# Install

`lampl` is one binary of about 22 MB. Pick the route that matches how you work. Every route installs the same 0.1.0 code.

## Shell installer

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/moneymikeMD/lamplighter/releases/latest/download/lampl-installer.sh | sh
lampl --version
```

Release archives exist for `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`. There is no Intel macOS or Windows build; use Cargo there. The installer places `lampl`
in `$CARGO_HOME/bin` (usually `~/.cargo/bin`); make sure that directory is on `PATH`. Checksums are published beside each archive. Run on this repo's 0.1.0 release with `CARGO_HOME` pointed at a
scratch directory and `INSTALLER_NO_MODIFY_PATH=1`, it downloaded the Apple Silicon archive and `lampl --version` printed `lampl 0.1.0`.

## Cargo

```sh
cargo install lampl
```

The workspace sets `rust-version = "1.99"`, so you need Rust 1.99 or newer. The build bundles SQLite and compiles the candle embedder, so expect a few minutes. Library crates are on crates.io too:
`lampl-core` (model, ranking, no I/O), `lampl-embed`, `lampl-store-pg`, `lampl-store-sqlite`, and a `lamplighter` name-holder that re-exports `lampl`.

## Docker

```sh
docker pull ghcr.io/moneymikemd/lamplighter
docker run --rm ghcr.io/moneymikemd/lamplighter --version
```

The image is public for linux/amd64 and linux/arm64, runs as a non-root user (uid 10001), and defaults to `lampl serve`. A running server needs a database and a keys file; use
[modes/docker-compose.md](modes/docker-compose.md).

## Claude Code plugin

```sh
claude plugin marketplace add moneymikeMD/moneymike-plugins
claude plugin install lamplighter@moneymike-plugins
```

The plugin installs hooks, an MCP registration and a skill. It does **not** install `lampl`: put the binary on `PATH` first and run `lampl init` (see [quickstart.md](quickstart.md)). The hook is a
Python script, so `python3` must be on `PATH` too. See [plugin.md](plugin.md).

## First run

A new client is not configured and does nothing until you choose a mode:

```sh
lampl init --local --prepare                                  # embedded store plus the one-time model download
lampl init --server http://127.0.0.1:7878 --api-key-file ~/.config/lampl/key   # or: use a server
```

`lampl doctor` reports what the client sees. The embedding model (about 1.1 GB) is downloaded from Hugging Face by `init --local --prepare` or `doctor --fetch-model` and by nothing else.

## Upgrade

Re-run the installer, `cargo install lampl --force`, or pull a newer image tag. A server applies pending migrations at start and refuses to start if the database schema is newer than the binary, so
upgrade the server first and then the clients: every response carries `X-Lampl-Api-Version`, and a client whose version differs stops with a message naming both numbers.

## Uninstall

Remove the binary (`rm "$(command -v lampl)"` or `cargo uninstall lampl`), then delete the config and data directories if you want the memories gone: `lampl.db` and `config.toml` live under the
platform data and config directories in a `lampl` folder (or `LAMPL_DATA_DIR` and `LAMPL_CONFIG_DIR`), and model weights under the platform cache directory in `lamplighter/models`. `claude plugin
uninstall lamplighter@moneymike-plugins` removes the plugin. Export first if you might return: `lampl export -o backup.ndjson` (server mode).
