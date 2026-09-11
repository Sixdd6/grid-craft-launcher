# CurseForge access for third-party launchers (2026-09-11)

How launchers get CurseForge access, and what works without an approved key. Items marked
VERIFY lack a live check.

## Options, ranked by legitimacy

### A. Apply for an official key (recommended)

Form: https://support.curseforge.com/support/solutions/articles/9000208346-about-the-curseforge-api-and-how-to-apply-for-a-key
It asks for contact details, a project description, links, and expected usage, and requires
accepting the third-party API terms
(https://support.curseforge.com/support/solutions/articles/9000207405-curse-forge-3rd-party-api-terms-and-conditions).
Overwolf answers by email in about 48 to 72 hours.

The key is a non-transferable secret. The terms forbid publishing it in source, sharing it, and
reaching the API through a proxy. Prism Launcher keeps its source key-free and injects the key
at build time through the `Launcher_CURSEFORGE_API_KEY` build flag from a CI secret
(https://prismlauncher.org/wiki/help-pages/apis/,
https://github.com/PrismLauncher/PrismLauncher/issues/263). Rate limits are not documented in
the application notes. VERIFY.

### B. The user pastes a personal key

Already supported through `CURSEFORGE_API_KEY` and the Settings screen. No waiting. The user
carries the terms.

### C. Public website endpoints and CDN patterns without a key

Since July 2026 file downloads from `edge.forgecdn.net` and the
`https://www.curseforge.com/api/v1/mods/{id}/files/{fileId}/download` route need a key
(https://blog.curseforge.com/introducing-api-key-authentication-for-curseforge-file-downloads/).
Which `www.curseforge.com/api/v1` endpoints still answer without one is unconfirmed. VERIFY.
Undocumented, no SLA, and blocks by IP. Not a product path.

### D. Proxies

`curse.tools` (community proxy) and the self-hosted `CurseForge.ApiProxy`
(https://github.com/CurseForgeCommunity/CurseForge.ApiProxy) hide a key behind a server. The
terms forbid proxy access, so both risk a takedown or a revoked key. Not a product path.

## Opt-out mods

Authors can disable third-party distribution (`allowModDistribution: false`, download URL
null). Prism opens the mod's page for a manual download and then matches the placed file by
fingerprint. The launcher already does the same (`ManualDownload`, `import_manual`).

## Recommendation

1. Apply for an official key as an open-source launcher. Keep the source key-free and inject it
   in the release workflow from a GitHub secret, the Prism way. The launcher's `config`
   already reads the key from the environment and from Settings, so a build-time default is a
   small addition: an `option_env!("GCL_CURSEFORGE_API_KEY")` fallback used only when neither
   the environment nor Settings has one.
2. Keep the personal-key path for development and for users who have their own key.
3. Do not implement C or D.

Sources: the links above and https://docs.curseforge.com/rest-api/.
