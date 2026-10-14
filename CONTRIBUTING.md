# Contributing to Ondalux Heatmap

Thanks for helping! You do not need to write Rust to make a difference.

## Ways to help

- **Report problems.** A plan that imports badly, a wall the detector
  misses, a number that looks wrong. Attach the floor plan or the `.lux`
  project when you can (remove anything confidential first).
- **Add access point models.** Every value must come from the vendor's
  official datasheet, with its URL in `source`. Values without a public
  source are not accepted. See [docs/ap-catalog.md](docs/ap-catalog.md).
- **Validate predictions.** Surveys of real buildings compared with the
  prediction help tune the default wall losses.
- **Test on macOS and Linux.** The scanning backends compile there but
  need testing on real hardware; see [docs/scanning.md](docs/scanning.md).
- **Translate** the app and reports (currently English and Spanish).
- **Write code.** Pick an issue and say you are working on it.

## Clean-room rule

Ondalux Heatmap is written from public knowledge only. Do not contribute
code, assets, data or text copied from commercial WiFi planning tools,
and do not use their names or trademarks in the project.

## Development

Requirements: Rust 1.82 or newer.

```sh
cargo run -p ondalux-app            # run the app
cargo test --workspace              # tests
cargo fmt --all                     # formatting
cargo clippy --workspace --all-targets -- -D warnings
```

Keep pull requests small and focused, with a clear description of what
changes for the user. Formatting, clippy and tests must pass. New
behavior needs a test; RF model changes need an update to
[docs/rf-models.md](docs/rf-models.md).

The website lives in [`website/`](website/README.md).

## Support

Questions that are not bugs: support@ondaluxheatmap.com

By contributing you agree that your contributions are licensed under the
MIT and Apache-2.0 licenses of this project.
