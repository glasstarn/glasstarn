# Contributing to glasstarn

Thank you for helping. glasstarn is a local process-mining engine with a
browser view, a command-line interface and an agent plugin. Contributions of
bug reports, test logs, process packs and code are welcome.

## Before you start

- Open an issue first for anything larger than a small fix, so we can agree on
  the approach.
- Never attach real company data to an issue or pull request. Reproduce with
  invented data, or describe the shape of the data (columns, row counts, the
  steps involved) instead.

## Build and test

You need Rust 1.88 or newer and Node.js 18 or newer (Node is only used for the
browser tests and the npm package).

```sh
cargo build --release -p pm-cli
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
node --test ui/tests/*.test.cjs
```

The end-to-end journeys drive the real application in a headless Chrome:

```sh
node tests/journeys/journeys.mjs
```

## What a good change looks like

- Every number shown to a person comes from the engine. The browser and the
  agent plugin word and lay out results; they do not calculate them.
- A fixed bug comes with a test that reproduces it.
- Expected results come from somewhere other than the code under test: a
  hand-built log, a constructed model, an invariant, or a reference tool.
  Do not record the program's own output as the expected answer.
- Results stay deterministic: the same data and settings give the same numbers.
- Plain words on screen: describe what happened to the work, not the algorithm.

## Process packs

A pack describes how a system stores one process: which tables to export, how
they link and which events they hold. See [docs/process-packs.md](docs/process-packs.md).
A new pack needs invented sample data that exercises it and a test that builds
the expected history from that data.

## Licence

By contributing you agree that your contribution is licensed under the
[Apache License 2.0](LICENSE).
