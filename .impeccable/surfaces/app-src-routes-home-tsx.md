---
version: 1
slug: "app-src-routes-home-tsx"
primary_target: "app/src/routes/Home.tsx"
related_targets: []
---

# Home (app shell + Up next + This week)

Scope: the desktop app's Home screen and the app shell (left rail, search, sync status) that every later screen inherits. Mode: Operate.

Audience/job: anime fans opening the app in the evening to resume the next unwatched episode in one click, and to see what's airing this week. Libraries of thousands of shows; must stay uncluttered (user's stated "feels wrong": too busy).

Constraints: SolidJS inside Tauri 2; dark scene; standard controls only; cover art is the only imagery; virtualized lists for big libraries; no torrents or downloaders anywhere.

## Direction contract

THESIS: Your library is your own broadcast schedule. Home reads like the evening's listings: what's on next, then the week set in a listings column rhythm. It refuses the streaming-service banner-and-rows wall.

OWN-WORLD: Graphite stepped tonal ramp (z0–z10, no ad-hoc greys) with newsprint-white type; listings red (#E5483B) for NEW/now/progress, amber for FINAL, teal for SUB/synced, violet as a fourth channel colour. Archivo for UI, Archivo Narrow uppercase for column heads, times and marks. 2px newsprint heading rules, 1px column rules, 4px radii. Signature: boxed glyph marks (NEW, FINAL, SUB), at most one per item, plus channel-coloured slot edges.

STORY: The visitor sees which episode is next for each show, plays it in one click, sees this week's releases at a glance, and trusts sync because every status shows its evidence.

FIRST VIEWPORT: 232px left rail (wordmark, nav with counts, sync evidence at the foot). Main: search bar and evening date line; "Up next" as 4 large episode cards (art, ep chip, mark, title, episode meta, red progress bar); "This week" as a 7-day listings grid with time rows, today underlined red, and watched slots struck through.

FORM: Broadcast Listings, assigned index 3 of my grounded list (1 obi strip, 2 manga spines, 3 TV listings, 4 cel layers, 5 ED credits, 6 mini-theater programme, 7 TV-guide poster calendar). seed:d87c469da2832169. Raises: stamped-not-hidden watched state; extent-sized A–Z rail; one cell grid; status with evidence; fixed tonal ramp.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
