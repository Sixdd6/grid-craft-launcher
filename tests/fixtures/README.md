# Fixtures

Recorded JSON from real services, one directory per source: `mojang/`, `fabric/`, `quilt/`,
`forge/`, `neoforge/`, `modrinth/`, `curseforge/`.

Record with `just record-fixture <source> <name> '<url>'`. Trim large arrays by hand to two or
three entries. Never commit a fixture that contains an API key or token.
