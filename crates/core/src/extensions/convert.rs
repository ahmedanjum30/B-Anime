//! Extensions in the single-manifest format, converted at install time.
//!
//! Such an extension is one JSON manifest (`type`, `payload` or `payloadURI`, `language`).
//! Online-stream and manga providers convert into an ordinary B-Anime extension: TypeScript is
//! transpiled, and the code is wrapped between a shim that recreates that runtime's globals (`fetch`
//! with its response shape, `LoadDoc`, `CryptoJS`, `$store`, …) and an adapter that maps its
//! `search` / `findEpisodes` / `findEpisodeServer` (or `findChapters` / `findChapterPages`) onto
//! B-Anime's methods. Torrent providers and plugins aren't supported.

use serde_json::Value;

use super::{ExtKind, Limits, MAX_CODE_BYTES, Manifest, download};
use crate::{Error, Result};

const SHIM: &str = include_str!("convert_shim.js");
const ADAPTER: &str = include_str!("convert_adapter.js");

/// Marks converted extensions in their manifest.
pub const ORIGIN: &str = "converted";

/// A single-file manifest: it has a `type` and carries (or points at) its code, and isn't ours.
pub fn is_manifest(v: &Value) -> bool {
    v.get("type").is_some_and(Value::is_string)
        && v.get("apiVersion").is_none()
        && (v.get("payload").is_some() || v.get("payloadURI").is_some())
}

/// The B-Anime kind for a converted extension's type, or why it can't be used.
pub fn kind(foreign_type: &str) -> std::result::Result<ExtKind, String> {
    match foreign_type {
        "onlinestream-provider" => Ok(ExtKind::Anime),
        "manga-provider" => Ok(ExtKind::Manga),
        "anime-torrent-provider" => Err("it's a torrent provider, which B-Anime doesn't support".into()),
        "plugin" => Err("it's a plugin; only online-streaming and manga providers convert".into()),
        "custom-source" => Err("it's a custom source; only online-streaming and manga providers convert".into()),
        other => Err(format!("unknown extension type {other:?}")),
    }
}

/// Their ids allow capitals and underscores; ours are lowercase with dashes.
pub fn convert_id(raw: &str) -> String {
    let mut id: String = raw
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    while id.contains("--") {
        id = id.replace("--", "-");
    }
    let id = id.trim_matches('-');
    let mut id: String = id.chars().take(48).collect();
    while id.ends_with('-') {
        id.pop();
    }
    if id.len() < 2 {
        id = format!("ext-{id}");
    }
    id
}

/// The manifest's `lang`: empty means English, "all" means several.
fn lang(v: &Value) -> String {
    match v.get("lang").and_then(Value::as_str).map(str::trim) {
        None | Some("") => "en".into(),
        Some("all") => "multi".into(),
        Some(l) => l.to_owned(),
    }
}

/// Strips TypeScript down to JavaScript (types, enums, parameter properties).
pub fn transpile(ts: &str) -> Result<String> {
    // The transformer asserts on input it doesn't expect; a bad extension mustn't take the app down.
    std::panic::catch_unwind(|| transpile_inner(ts))
        .unwrap_or_else(|_| Err(Error::Extension("the extension's TypeScript couldn't be converted".into())))
}

fn transpile_inner(ts: &str) -> Result<String> {
    use oxc::allocator::Allocator;
    use oxc::codegen::Codegen;
    use oxc::parser::Parser;
    use oxc::semantic::SemanticBuilder;
    use oxc::span::SourceType;
    use oxc::transformer::{TransformOptions, Transformer};

    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, ts, SourceType::ts()).parse();
    if !parsed.diagnostics.is_empty() {
        let first = parsed.diagnostics.first().map(|d| d.to_string()).unwrap_or_else(|| "parse failed".into());
        return Err(Error::Extension(format!("the extension's TypeScript doesn't compile: {first}")));
    }
    let mut program = parsed.program;
    let scoping = SemanticBuilder::new().with_excess_capacity(2.0).with_enum_eval(true).build(&program).semantic.into_scoping();
    let out = Transformer::new(&allocator, std::path::Path::new("extension.ts"), &TransformOptions::default())
        .build_with_scoping(scoping, &mut program);
    if let Some(d) = out.diagnostics.first() {
        return Err(Error::Extension(format!("the extension's TypeScript doesn't compile: {d}")));
    }
    Ok(Codegen::new().build(&program).code)
}

/// Wraps a provider's JavaScript so it runs as a B-Anime extension.
pub fn wrap(js: &str, kind: ExtKind, user_config: Option<&Value>) -> String {
    let config = user_config.map(Value::to_string).unwrap_or_else(|| "null".into());
    let kind = match kind {
        ExtKind::Anime => "anime",
        ExtKind::Manga => "manga",
    };
    format!(
        "// Converted extension, wrapped by B-Anime.\n\
         const __extConfig = {config};\nconst __extKind = \"{kind}\";\n{SHIM}\n\
         const __InnerProvider = (function () {{\n{js}\n;return typeof Provider === \"function\" ? Provider : undefined;\n}}).call(globalThis);\n\
         {ADAPTER}"
    )
}

/// Converts a single-file manifest (already downloaded from `manifest_url`) into ours plus code.
pub async fn convert(raw: &Value, manifest_url: &reqwest::Url, limits: Limits) -> Result<(Manifest, String)> {
    let s = |k: &str| raw.get(k).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_owned();
    let name = s("name");
    let kind = kind(&s("type")).map_err(|why| Error::Extension(format!("{} can't be used: {why}", if name.is_empty() { "This extension" } else { &name })))?;
    if raw.get("isDevelopment").and_then(Value::as_bool) == Some(true) {
        return Err(Error::Extension("development-mode extensions can't be installed".into()));
    }
    let id = convert_id(&s("id"));

    let payload_uri = s("payloadURI");
    let mut code = s("payload");
    let mut code_url = manifest_url.to_string();
    if code.is_empty() {
        if payload_uri.is_empty() {
            return Err(Error::Extension("the manifest has no code (payload or payloadURI)".into()));
        }
        let url = manifest_url.join(&payload_uri).map_err(|e| Error::Extension(format!("bad payloadURI: {e}")))?;
        code = download(&url, MAX_CODE_BYTES, limits).await?;
        code_url = url.to_string();
    }
    if code.len() > MAX_CODE_BYTES {
        return Err(Error::Extension(format!("the extension's code is larger than {} KB", MAX_CODE_BYTES / 1024)));
    }
    let js = match s("language").to_ascii_lowercase().as_str() {
        "typescript" => transpile(&code)?,
        "javascript" | "" => code,
        other => return Err(Error::Extension(format!("extensions written in {other} can't run in B-Anime"))),
    };

    let manifest = Manifest {
        id,
        name: if name.is_empty() { s("id") } else { name },
        version: { let v = s("version"); if v.is_empty() { "0.0.0".into() } else { v } },
        kind,
        author: s("author"),
        description: s("description"),
        language: Some(lang(raw)),
        icon: Some(s("icon")).filter(|i| !i.is_empty()),
        code_url,
        api_version: super::API_VERSION,
        origin: Some(ORIGIN.into()),
    };
    Ok((manifest, wrap(&js, kind, raw.get("userConfig"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::runtime;
    use serde_json::json;

    #[test]
    fn ids_and_kinds_convert() {
        assert_eq!(convert_id("AnimePahe"), "animepahe");
        assert_eq!(convert_id("my_source.v2"), "my-source-v2");
        assert_eq!(convert_id("x"), "ext-x");
        assert!(kind("anime-torrent-provider").is_err());
        assert_eq!(kind("manga-provider").unwrap(), ExtKind::Manga);
        assert!(is_manifest(&json!({ "type": "manga-provider", "payload": "" })));
        assert!(!is_manifest(&json!({ "type": "x", "payload": "", "apiVersion": 1 })));
    }

    #[test]
    fn typescript_becomes_javascript() {
        let js = transpile("enum Q { Low = 1 } type T = { a: string }; class Provider { constructor(private x: number = Q.Low) {} get(): T { return { a: String(this.x) } } }").unwrap();
        assert!(!js.contains(": number") && js.contains("class Provider"), "{js}");
        assert!(transpile("class {").is_err());
    }

    /// An online-stream provider in that format, using the runtime's globals.
    const ANIME_TS: &str = r#"
        /// <reference path="./online-streaming-provider.d.ts" />
        class Provider {
            api = "https://example.invalid"
            getSettings(): Settings {
                return { episodeServers: ["broken", "main"], supportsDub: true }
            }
            async search(opts: SearchOptions): Promise<SearchResult[]> {
                const $ = LoadDoc(`<div class="r" data-id="show-1"><a href="/show-1">${opts.media.romajiTitle}</a></div>
                                   <div class="r" data-id="show-1-dub"><a href="/show-1-dub">Dubbed</a></div>`)
                return $(".r").map((i, el) => ({
                    id: el.data("id")!,
                    title: el.find("a").text(),
                    url: this.api + el.find("a").attr("href"),
                    subOrDub: el.data("id")!.endsWith("dub") ? "dub" : "sub",
                }))
            }
            async findEpisodes(id: string): Promise<EpisodeDetails[]> {
                return [1, 2].map(n => ({ id: `${id}-ep${n}`, number: n, url: `${this.api}/${id}/${n}`, title: `Episode ${n}` }))
            }
            async findEpisodeServer(episode: EpisodeDetails, server: string): Promise<EpisodeServer> {
                if (server === "broken") throw new Error("server down")
                const key = CryptoJS.enc.Utf8.parse("0123456789abcdef")
                const iv = CryptoJS.enc.Utf8.parse("fedcba9876543210")
                const sealed = CryptoJS.AES.encrypt(episode.url, key, { iv }).toString()
                const url = CryptoJS.AES.decrypt(sealed, key, { iv }).toString(CryptoJS.enc.Utf8)
                const u = new URL(url + "/master.m3u8")
                u.searchParams.set("t", Buffer.from("tok").toString("base64"))
                return {
                    server,
                    headers: { Referer: this.api },
                    videoSources: [{ url: u.toString(), type: "m3u8", quality: "1080p", subtitles: [{ id: "en", url: "https://example.invalid/en.vtt", language: "English", isDefault: true }] }],
                }
            }
        }
    "#;

    fn anime_code() -> String {
        wrap(&transpile(ANIME_TS).unwrap(), ExtKind::Anime, None)
    }

    #[tokio::test]
    async fn an_online_stream_provider_runs_through_the_adapter() {
        let code = anime_code();
        let limits = Limits::default();
        let methods = runtime::call(&code, "__methods", &json!([]), limits).await.unwrap().value;
        for m in ["search", "episodes", "sources"] {
            assert!(methods.as_array().unwrap().iter().any(|x| x == m), "{methods}");
        }

        let q = json!([{ "query": "Frieren", "dub": false, "media": { "id": 154587, "titles": ["Sousou no Frieren", "Frieren"] } }]);
        let found = runtime::call(&code, "search", &q, limits).await.unwrap().value;
        assert_eq!(found, json!([{ "id": "show-1", "title": "Sousou no Frieren" }]));
        let dub = runtime::call(&code, "search", &json!([{ "query": "x", "dub": true }]), limits).await.unwrap().value;
        assert_eq!(dub[0]["id"], "show-1-dub");

        let eps = runtime::call(&code, "episodes", &json!(["show-1"]), limits).await.unwrap().value;
        assert_eq!(eps[1]["number"], 2);
        let ep_id = eps[1]["id"].as_str().unwrap().to_owned();

        let out = runtime::call(&code, "sources", &json!([ep_id]), limits).await.unwrap();
        assert_eq!(out.value["headers"]["Referer"], "https://example.invalid");
        assert_eq!(out.value["sources"][0]["type"], "hls");
        assert_eq!(out.value["sources"][0]["url"], "https://example.invalid/show-1/2/master.m3u8?t=dG9r");
        assert_eq!(out.value["sources"][0]["label"], "main · 1080p");
        assert_eq!(out.value["subtitles"][0], json!({ "url": "https://example.invalid/en.vtt", "lang": "en", "label": "English" }));
        assert!(out.logs.iter().any(|l| l.contains("server down")), "{:?}", out.logs);
    }

    #[tokio::test]
    async fn dub_searches_never_return_subs() {
        let provider = |supports_dub: bool| {
            format!(
                r#"class Provider {{
                    getSettings() {{ return {{ episodeServers: [], supportsDub: {supports_dub} }} }}
                    async search() {{ return [{{ id: "a", title: "A", url: "", subOrDub: "sub" }}, {{ id: "b", title: "B", url: "" }}] }}
                    async findEpisodes() {{ return [] }}
                    async findEpisodeServer() {{ return {{ videoSources: [] }} }}
                }}"#
            )
        };
        let limits = Limits::default();
        let ids = |v: Value| v.as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
        let no_dubs = wrap(&provider(false), ExtKind::Anime, None);
        assert!(ids(runtime::call(&no_dubs, "search", &json!([{ "query": "x", "dub": true }]), limits).await.unwrap().value).is_empty());
        assert_eq!(ids(runtime::call(&no_dubs, "search", &json!([{ "query": "x", "dub": false }]), limits).await.unwrap().value), ["a", "b"]);
        // A source with dubs, but none for this show: no fallback to the sub.
        let with_dubs = wrap(&provider(true), ExtKind::Anime, None);
        assert!(ids(runtime::call(&with_dubs, "search", &json!([{ "query": "x", "dub": true }]), limits).await.unwrap().value).is_empty());
        assert_eq!(ids(runtime::call(&with_dubs, "search", &json!([{ "query": "x", "dub": false }]), limits).await.unwrap().value), ["a", "b"]);
    }

    #[tokio::test]
    async fn a_manga_provider_runs_through_the_adapter() {
        let js = r#"
            class Provider {
                getSettings() { return { supportsMultiLanguage: false, supportsMultiScanlator: false } }
                async search(opts) { return [{ id: "m1", title: opts.query.toUpperCase(), year: opts.year }] }
                async findChapters(id) { return [{ id: id + "-c1", url: "", title: "One", chapter: "1", index: 0 }, { id: id + "-c15", url: "", title: "Half", chapter: "1.5", index: 1 }] }
                async findChapterPages(id) { return [{ url: "https://x.invalid/2.jpg", index: 1, headers: { Referer: "https://x.invalid" } }, { url: "https://x.invalid/1.jpg", index: 0, headers: { Referer: "https://x.invalid" } }] }
            }"#;
        let code = wrap(js, ExtKind::Manga, None);
        let limits = Limits::default();
        let found = runtime::call(&code, "search", &json!([{ "query": "berserk", "year": 1989 }]), limits).await.unwrap().value;
        assert_eq!(found, json!([{ "id": "m1", "title": "BERSERK", "year": 1989 }]));
        let ch = runtime::call(&code, "chapters", &json!(["m1"]), limits).await.unwrap().value;
        assert_eq!(ch[1], json!({ "id": "m1-c15", "number": 1.5, "title": "Half" }));
        let pages = runtime::call(&code, "pages", &json!(["m1-c1"]), limits).await.unwrap().value;
        assert_eq!(pages["pages"][0]["url"], "https://x.invalid/1.jpg");
        assert_eq!(pages["headers"]["Referer"], "https://x.invalid");
    }

    #[tokio::test]
    async fn shim_globals_behave() {
        let js = r#"
            class Provider {
                getSettings() { return {} }
                async search() {
                    const $ = LoadDoc('<ul><li><b>1</b></li><li class="x">2</li><li>3</li></ul>');
                    // `openssl enc -aes-256-cbc -md md5 -pass pass:secret`, as CryptoJS writes passphrase output.
                    const salted = "U2FsdGVkX18BAgMEBQYHCDgayERkxXibMuUh6ooXMww=";
                    $store.set("k", { a: 1 });
                    const hex = CryptoJS.enc.Hex.stringify(CryptoJS.enc.Utf8.parse("hi"));
                    return [{
                        count: $("li").length(),
                        nextOfFirst: $("li").first().next().text(),
                        notX: $("li").not(".x").map((i, e) => e.text()),
                        closest: $("b").closest("ul").children().length(),
                        has: $("li").has("b").length(),
                        siblings: $(".x").siblings().length(),
                        hex,
                        b64: Buffer.from("aGVsbG8=", "base64").toString(),
                        store: $store.get("k").a,
                        str: $toString({ x: 1 }),
                        salted: CryptoJS.AES.decrypt(salted, "secret").toString(CryptoJS.enc.Utf8),
                        params: new URLSearchParams({ q: "a b", n: 2 }).toString(),
                    }];
                }
                async findEpisodes() { return [] }
                async findEpisodeServer() { return { videoSources: [] } }
            }"#;
        // The adapter reshapes search results, so read the inner provider's own answer.
        let probe = format!("{}\nProvider.prototype.raw = async function () {{ return this.inner.search({{}}); }};", wrap(js, ExtKind::Anime, None));
        let v = runtime::call(&probe, "raw", &json!([]), Limits::default()).await.unwrap().value;
        assert_eq!(
            v[0],
            json!({
                "count": 3, "nextOfFirst": "2", "notX": ["1", "3"], "closest": 3, "has": 1, "siblings": 2,
                "hex": "6869", "b64": "hello", "store": 1, "str": "{\"x\":1}", "salted": "hello world",
                "params": "q=a+b&n=2"
            })
        );
    }

    #[tokio::test]
    async fn unsupported_types_are_refused_with_a_reason() {
        let raw = json!({ "id": "t", "name": "Torrents", "type": "anime-torrent-provider", "language": "javascript", "payload": "class Provider {}" });
        let url = reqwest::Url::parse("https://example.invalid/m.json").unwrap();
        let err = convert(&raw, &url, Limits::default()).await.unwrap_err();
        assert!(err.to_string().contains("torrent provider"), "{err}");
    }
}
