# Third-party code

No third-party code has been ported. Dependencies are listed in `Cargo.lock` and checked by
cargo-deny (`just deny`).

Checked with `grep -rIn "ported from\|Copyright" --exclude-dir=target --exclude-dir=.git --exclude=Cargo.lock --exclude=LICENSE .` (2026-09-06):
the only matches are this file's own instructions, the README's statement of the policy, and the
axodotdev copyright header inside the cargo-dist-generated `.github/workflows/release.yml`, which
is that tool's own file, not launcher code.
