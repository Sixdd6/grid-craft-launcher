# Description markup usage (2026-09-08)

Sample: 15 of the top 40 Modrinth mods by downloads (`/v2/search?index=downloads`), bodies read
from `GET /v2/project/{id}`. Counts are bodies using the feature.

| Feature | Bodies | Example |
|---|---|---|
| Headings (`#` or `<h1..6>`) | 15/15 | all |
| Links (`[t](u)`, `<a>`) | 15/15 | all |
| Bold (`**`, `<strong>`, `<b>`) | 14/15 | Sodium |
| Lists (`-`, `*`, `+`, `<ul>`) | 13/15 | Fabric API |
| Images (`![]()`, `<img>`) | 12/15 | Sodium |
| Badges (image inside a link) | 11/15 | Sodium |
| Layout tags (`<br>`, `<div>`, `<span>`) | 11/15 | Iris |
| Alignment (`<center>`, `<div align>`) | 8/15 | Sodium |
| Italic (`*`, `_`, `<em>`) | 8/15 | Lithium |
| Horizontal rules (`---`, `<hr>`) | 7/15 | Create |
| Inline code | 5/15 | Fabric API |
| Fenced code / `<pre>` | 3/15 | Fabric API |
| Ordered lists | 2/15 | JEI |
| Pipe tables | 1/15 | ImmediatelyFast |
| `<details>`/`<summary>` | 1/15 | Mod Menu |
| Blockquotes, setext headings | 0/15 | none |

Also seen: `<div class="spoiler">` (Xaero's World Map), inline `style` attributes, `target`/`rel`
on links. The ImmediatelyFast table:

```
| Other mods | Without ImmediatelyFast | With ImmediatelyFast | Improvement |
|------------|-------------------------|----------------------|-------------|
| None       | 16 FPS                  | 60 FPS               | 3.75x       |
```

## Changelogs

- Modrinth: each item of `GET /v2/project/{id}/version` carries `changelog` (markdown string)
  and `changelog_url` (usually null). Confirmed live on Sodium (`AANobbMI`). Our `RawVersion`
  does not read it yet.
- CurseForge: `GET /v1/mods/{modId}/files/{fileId}/changelog` returns `{"data": "<html>"}` per
  https://docs.curseforge.com/rest-api/. Needs `x-api-key`. VERIFY (no key on this machine).

## Renderer constraints

Slint `Text` has no inline spans, so bold and italic inside a paragraph are stripped. Block
kinds worth adding: table, rule, image, quote, nested bullet depth, details as heading +
content, bold-only paragraph as a subheading.
