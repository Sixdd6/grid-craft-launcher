# Fixtures

Recorded JSON from real services, one directory per source: `mojang/`, `fabric/`, `quilt/`,
`forge/`, `neoforge/`, `modrinth/`, `curseforge/`.

Record with `just record-fixture <source> <name> '<url>'`. Trim large arrays by hand to two or
three entries. Never commit a fixture that contains an API key or token.

`launch/` holds a plain-text excerpt of a real game log, not JSON: one log4j event captured
from a 26.2 Fabric run, used by the `launch::log4j` and `launch::crash` parsers' tests.
