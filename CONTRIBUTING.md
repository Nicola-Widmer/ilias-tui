# Contributing

## Development

```bash
cargo test
```

```bash
cargo clippy --all-targets -- -D warnings
```

```bash
cargo fmt
```

CI (GitHub Actions) runs the tests, clippy and `cargo fmt --check` on macOS, Linux and
Windows, and `cargo audit`. No test reaches the network: parser tests use small, public
HSLU pages in `tests/fixtures/` and anonymised snippets, client tests a fake ILIAS. Never
commit real logged-in pages; they contain personal data.

To try features against a real server, there is a throwaway ILIAS 10 in Docker, see
[dev/README.md](dev/README.md). Try anything that writes only there, or with
`ILIAS_DRY_RUN=1`.

- [docs/architecture.md](docs/architecture.md): the code layout, the component
  architecture and how to add a feature (keep files under about 600 lines).
- [CONTEXT.md](CONTEXT.md): the words the code and docs use.
- [docs/roadmap.md](docs/roadmap.md): what is missing, and the rules for anything that
  changes data on ILIAS.
- [docs/security.md](docs/security.md): what is stored, sent and opened.

## Versions

ilias-tui uses [semantic versioning](https://semver.org). The version is in `Cargo.toml`;
`ilias-tui --version`, the top bar and the help (`?`) show it. Before 1.0, the middle number
goes up for new features and changes in behaviour (0.2.0 → 0.3.0), the last one for fixes
only (0.2.0 → 0.2.1). [CHANGELOG.md](CHANGELOG.md) lists what changed in each version.

To release a version:

1. Add what changed under "Unreleased" in `CHANGELOG.md` as you go.
2. Pick the new version, rename "Unreleased" to it with the date, and add a new empty
   "Unreleased" above.
3. Set the same version in `Cargo.toml` and run `cargo build` (it updates `Cargo.lock`).
   A test fails while the changelog has no section for the version in `Cargo.toml`.
4. Commit, then tag the commit, e.g. `git tag v0.3.0`, and push the tag.
