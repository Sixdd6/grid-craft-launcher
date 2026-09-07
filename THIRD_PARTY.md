# Third-party code

No third-party code has been ported. Dependencies are listed in `Cargo.lock` and checked by
cargo-deny (`just deny`).

Checked with `grep -rIn "ported from\|Copyright" --exclude-dir=target --exclude-dir=.git --exclude=Cargo.lock --exclude=LICENSE .` (2026-09-06):
the matches are the README's statement of the policy, the axodotdev copyright header inside the
cargo-dist-generated `.github/workflows/release.yml` (that tool's own file, not launcher code),
a planning doc (`docs/superpowers/plans/2026-09-06-ai-toolchain-scaffold.md`) that quotes the
README's policy wording and this file's own check instructions, and this file's own instructions.
None of these is ported code.
