# Contributing

Thanks for helping out. This project is spec-driven with OpenSpec: see [AGENTS.md](AGENTS.md).

## Before you open a PR
- `cargo test` and `cargo clippy --all-targets -- -D warnings` must pass.
- Propose non-trivial changes as an OpenSpec change (`openspec/changes/`) first.
- Update `docs/architecture.md` if structure, request flow or component status changes.

## License and sign-off (DCO)
The project is licensed under [Apache-2.0](LICENSE). By contributing you agree your
contribution is licensed under the same terms.

Every commit must carry a `Signed-off-by` line certifying the
[Developer Certificate of Origin](https://developercertificate.org/):

    git commit -s -m "Your message"

which appends `Signed-off-by: Your Name <you@example.com>`. Use your real name. PRs with
unsigned commits will be asked to amend (`git rebase --signoff`).
