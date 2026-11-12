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
  // query: { query: string, dub: boolean }
  // `dub` is true when the user picked "Dub" (English audio). Return the dubbed entries if the
  // site lists them separately; ignore it if the site has no dubs.
  // → [{ id, title, image?, year? }]
  async search(query) {}

  // id: a search result's id
  // → [{ id, number, title? }]
  async episodes(id) {}

  // episodeId: an episode's id
  // → { headers?, sources: [{ url, type: "hls" | "mp4", quality?, label?, audio? }],
  //     subtitles?: [{ url, lang, label?, format?: "vtt" | "ass" | "srt" }] }
  // `audio` ("ja"/"sub" or "en"/"dub") marks a source's audio language when an episode offers
  // both; the player starts on the user's preferred one and shows an Audio menu.
  // HLS streams with alternate audio tracks need nothing extra: the player lists those itself.
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

- `fetch(url, { method, headers, body, redirect })` →
  `{ ok, status, statusText, url, redirected, headers.get(name), headers.getSetCookie(), text(), json() }`.
  http and https only. Requests to this computer or private networks are refused. Bodies are text
  and capped at 8 MB. Redirects are followed (up to 5) unless `redirect` is `"manual"` or
  `"error"`. At most 60 requests per call.
- `console.log / info / warn / error`: shown in the Extensions screen's tester.
- `atob`, `btoa`, `URL`, `URLSearchParams`, `TextEncoder`, `TextDecoder`, `setTimeout`,
  `clearTimeout`, plus the standard library (`JSON`, `RegExp`, `Map`, `Promise`, …).

Not available: the DOM, Node.js APIs, file access. Bundle any libraries you need (an HTML
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

B-Anime's own repository, [`docs/repository.json`](repository.json), is always included and has
its own **B-Anime** panel at the top of **Browse**. It lists extensions made by the project, which
only use legal, freely available sources. To publish one there, put its `manifest.json` and
`main.js` in `docs/sources/<id>/` and add an entry to that file with
`"manifestUrl": "sources/<id>/manifest.json"`; installed copies of B-Anime see it as soon as
it's on the `main` branch.

## Testing your extension locally

Development builds of B-Anime can install an extension straight from files on your computer,
so you can test changes without uploading them anywhere. Release builds can't: they only
install from `https://` addresses.

1. Run B-Anime from the source code: `cd app` then `npm run tauri dev`.
2. Go to **Extensions → Installed** and choose **Install from a file (development build)**.
3. Pick your manifest `.json`. B-Anime reads the code from the same folder: the file named in
   `codeUrl` (`"codeUrl": "main.js"`) or, in the converted format, the code in `payload` or the
   file named in `payloadURI`. A full `http(s)` address in either field is downloaded instead.
4. Edit your files, then press **Reload** on the extension to read them from disk again. Use
   **Try it** on the same row to run a search and see the results and your `console.log` output.

A local extension shows **Local file** in its details, and the update check skips it. It runs in
the same sandbox as any other extension: the files are read once, at install or reload, and the
extension itself still can't read your files or reach this computer or your local network.

## Converted extensions

B-Anime also installs extensions written in a second format and converts them when they're
installed. In that format, an extension is a single JSON manifest with a `type` and its code in
`payload` (or at `payloadURI`). Paste such a manifest URL to install it, or add a repository of
them: a marketplace list of manifests (each with a `manifestURI`) or `{ "urls": [...] }`. The
Extensions screen marks them as converted.

- **Supported:** `onlinestream-provider` (shows up as an anime source) and `manga-provider`, in
  JavaScript or TypeScript. TypeScript is compiled to JavaScript on install.
- **Not supported:** `anime-torrent-provider`, `plugin`, `custom-source`, and extensions written in
  Go. Repositories leave these out of their listing, and installing one directly says why.
- **What the converted code gets:** a `fetch` with that format's response shape (synchronous
  `text()` and `json()`, `headers`, `cookies`), `LoadDoc` and `Doc` (goquery-style selections),
  `CryptoJS` (AES-CBC and the `enc` encoders, including OpenSSL `Salted__` passphrases), `Buffer`,
  `FormData`, `$store`, `$toString`, `$clone`, `$replace`, `$unmarshalJSON` and
  `$getUserPreference`. `$getUserPreference` returns the default from `userConfig`.
- **Limitations:**
  - Every call runs in a fresh sandbox, so `$store` doesn't survive between calls.
  - `ChromeDP`, `$habari`, `$sleep` and `$await` aren't available.
  - `fetch` doesn't impersonate a browser.
  - The same sandbox limits apply: no local-network requests, 30 seconds and 60 requests per call.
- **Episodes and servers:** episode ids carry the provider's whole episode object, because
  `findEpisodeServer` needs it. For video, servers are tried in the order `getSettings()` lists
  them, and the first that returns video wins.

## Example

[`docs/sources/mock-heaven`](sources/mock-heaven) is a learning extension that reads a static
mock site. It's listed in B-Anime's own repository, or install it by pasting its manifest URL:

```
https://raw.githubusercontent.com/ahmedanjum30/B-Anime/main/docs/sources/mock-heaven/manifest.json
```
