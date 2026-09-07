# Forge processor output hashes differ on a zlib-ng host

Checked 2026-09-07 against Forge 47.4.10 for Minecraft 1.20.1, on Nobara (Fedora 43).

## Symptom

`gcl loader install --loader forge` failed after the `jarsplitter` processor with
`processor output <root>/cache/libraries/net/minecraft/client/1.20.1-20230612.114412/client-1.20.1-20230612.114412-extra.jar missing or hash mismatch`.
The file was on disk and complete.

## Cause

`install_profile.json` declares each processor's outputs as `path -> sha1`. For `jarsplitter`
those are `MC_EXTRA_SHA` and `MC_SLIM_SHA`, the sha1s of the **compressed bytes** of the jars
Forge's own build produced.

`jarsplitter` writes those jars with `java.util.zip`, which deflates through `libzip.so` inside
the JRE. Mojang's `java-runtime-gamma` build links that library against the host's
`/lib64/libz.so.1` rather than bundling zlib. On Fedora, Nobara and Arch, `libz.so.1` is
zlib-ng's compatibility build. zlib-ng emits a different deflate stream than stock zlib for the
same input at the same level, so the jar bytes differ and the sha1 differs.

Evidence gathered by the rust-diagnostics agent on `client-<mc>-extra.jar`:

| what | result |
|---|---|
| sha1 of our jar vs `MC_EXTRA_SHA` | differ |
| entries in each archive | 13 517 |
| entries whose name, order, CRC32 and uncompressed size match | 13 517 (all) |
| bytes of the deflate streams | differ |

The archives are content-identical. Only the compressed representation differs. `MC_SLIM_SHA`
behaves the same way. Compressing the same input with stock zlib and with zlib-ng at the same
level gives two different streams, which is what the repro test in
`crates/gcl-core/src/loaders/processors_tests.rs` stands in for: it writes one zip entry at
deflate level 6 and at level 9 and asserts the launcher accepts the second against the first's
hash.

## What we do

`loaders::processors` splits the two cases that were folded into one:

- **Missing output**: still fatal, `Error::ProcessorOutput`, message says "is missing".
- **Hash mismatch on an existing output**: open it with the `zip` crate and read every entry end
  to end, which checks each entry's CRC32 against its header. If the archive reads cleanly it is
  accepted with a `tracing::warn!` naming the expected and actual sha1. If it does not,
  `Error::ProcessorOutputDamaged` fails the install with "hash mismatch and the archive is
  damaged".

An accepted output's real sha1 is written to `<output>.sha1`. The pre-run skip accepts an output
matching the profile hash **or** that sidecar, so a second install does not rerun the processors.

## Not the cause

- Not a partial download: the vanilla client jar the processor reads hashes correctly.
- Not a wrong Java: the same mismatch appears with `java-runtime-gamma` and with the system JDK
  21 on the same host, both of which resolve `libz.so.1` to zlib-ng.
- Not Forge publishing a wrong hash: the jar Forge hashed and the jar we produced hold the same
  13 517 entries with the same CRC32s.
