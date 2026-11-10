# Writing a B-Anime extension

Extensions add sources for watching anime and reading manga online. B-Anime ships with none:
users install the ones they choose, from a URL or from a repository they add. You're responsible
for what your extension accesses; respect the terms of the sites you use.

An extension is two files on any web host:

- `manifest.json`: what it is.
- A script (e.g. `main.js`) that defines `class Provider`.

Users install it by pasting the manifest's URL into **Extensions → Install from URL**.

## Manifest

```json
{
  "id": "my-source",
  "name": "My Source",
  "version": "1.0.0",
  "kind": "anime",
  "author": "you",
  "description": "One sentence about what it provides.",
  "language": "en",
  "icon": "https://example.com/icon.png",
  "codeUrl": "main.js",
  "apiVersion": 1
}
```

| Field | Required | Notes |
|---|---|---|
| `id` | yes | Lowercase letters, digits and dashes, 2–48 characters. Never change it. |
| `name`, `version` | yes | Bump `version` to ship an update; B-Anime compares versions numerically. |
| `kind` | yes | `"anime"` or `"manga"`. |
| `codeUrl` | yes | The script. Relative URLs resolve against the manifest's URL. Max 4 MB. |
| `apiVersion` | yes | `1`. |
| `author`, `description`, `language`, `icon` | no | Shown in the Extensions screen. |

Updates are checked at the manifest URL the user installed from.

## Provider (anime)

```js
class Provider {
  // query: { query: string }
  // → [{ id, title, image?, year? }]
  async search(query) {}

  // id: a search result's id
  // → [{ id, number, title? }]
  async episodes(id) {}

  // episodeId: an episode's id
  // → { headers?, sources: [{ url, type: "hls" | "mp4", quality?, label? }],
  //     subtitles?: [{ url, lang, label?, format?: "vtt" | "ass" | "srt" }] }
  async sources(episodeId) {}
}
```

## Provider (manga)

```js
class Provider {
  async search(query) {}            // → [{ id, title, image?, year? }]
  async chapters(id) {}             // → [{ id, number, title?, volume? }]
  async pages(chapterId) {}         // → { headers?, pages: [{ url }] }
}
```

`headers` (an object) are sent with every video/page request for that result, e.g. a `Referer`.

## The sandbox

Each call runs in a fresh, isolated JavaScript engine (QuickJS, modern ES2023 syntax). Nothing
carries over between calls. Available:

- `fetch(url, { method, headers, body })` → `{ ok, status, url, headers.get(name), text(), json() }`.
  http and https only. Requests to this computer or private networks are refused. Bodies are text
  and capped at 8 MB. Redirects are followed (up to 5). At most 60 requests per call.
- `console.log / info / warn / error`: shown in the Extensions screen's tester.
- `atob`, `btoa`, plus the standard library (`JSON`, `RegExp`, `Map`, `Promise`, …).

Not available: the DOM, Node.js APIs, timers, file access. Bundle any libraries you need (an HTML
parser, crypto) into your script, e.g. with esbuild: `esbuild src.js --bundle --format=iife`.
Make sure `class Provider` ends up at the top level of the output.

Limits per call: 30 seconds, 96 MB of memory. A call that exceeds them is stopped.

## Repositories

A repository is a JSON array of manifest URLs, or of objects with a `manifestUrl` and optional
`name`, `kind`, `version`, `author`, `description` for the listing:

```json
[
  "https://example.com/my-source/manifest.json",
  { "manifestUrl": "other/manifest.json", "name": "Other Source", "kind": "manga" }
]
```

Users add repositories in **Extensions → Repositories**.

## Example

[`docs/examples/demo-streams`](examples/demo-streams) serves public test videos for any search.
Install it with:

```
https://raw.githubusercontent.com/ahmedanjum30/B-Anime/main/docs/examples/demo-streams/manifest.json
```
