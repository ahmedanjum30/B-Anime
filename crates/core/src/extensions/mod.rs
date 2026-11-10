//! Extensions: third-party JavaScript providers for online streaming and manga sources.
//!
//! An extension is a manifest (JSON) plus one script defining `class Provider`. B-Anime ships
//! none; users install them from a URL or from repositories they add. See docs/extensions.md.
//! Extensions in the single-manifest format (online-stream and manga providers) convert on
//! install; see convert.rs.

pub mod host;
pub mod runtime;
pub mod convert;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::Db;
use crate::{Error, Result};
pub use runtime::{CallOutput, Limits};

pub const API_VERSION: u32 = 1;
const MAX_CODE_BYTES: usize = 4 * 1024 * 1024;
/// Single-file manifests can carry their code inline.
const MAX_MANIFEST_BYTES: usize = MAX_CODE_BYTES + 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtKind {
    Anime,
    Manga,
}

impl ExtKind {
    /// Methods a provider of this kind must have.
    pub fn required_methods(self) -> &'static [&'static str] {
        match self {
            ExtKind::Anime => &["search", "episodes", "sources"],
            ExtKind::Manga => &["search", "chapters", "pages"],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: ExtKind,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    /// Where the script lives; relative URLs resolve against the manifest's URL.
    pub code_url: String,
    pub api_version: u32,
    /// "converted" for extensions converted from the single-manifest format; absent for ours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    #[serde(flatten)]
    pub manifest: Manifest,
    /// The manifest URL it was installed from; updates are checked there.
    pub source_url: String,
    pub enabled: bool,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Valid ids: lowercase letters, digits and dashes, 2–48 characters.
fn valid_id(id: &str) -> bool {
    (2..=48).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

/// Numeric-aware version comparison: "1.10.0" > "1.9.2".
pub fn newer(candidate: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split(|c: char| !c.is_ascii_digit()).filter_map(|p| p.parse().ok()).collect() };
    parts(candidate) > parts(current)
}

async fn download(url: &reqwest::Url, cap: usize, limits: Limits) -> Result<String> {
    runtime::check_url(url, limits.allow_private_hosts).map_err(Error::Extension)?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("B-Anime/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| Error::Network(e.to_string()))?;
    let mut res = client.get(url.clone()).send().await.map_err(|e| Error::Network(e.to_string()))?;
    if !res.status().is_success() {
        return Err(Error::Extension(format!("{url} answered HTTP {}", res.status().as_u16())));
    }
    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await.map_err(|e| Error::Network(e.to_string()))? {
        if body.len() + chunk.len() > cap {
            return Err(Error::Extension(format!("{url} is larger than {} KB", cap / 1024)));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| Error::Extension(format!("{url} isn't UTF-8 text")))
}

/// Downloads and checks an extension without installing it.
pub async fn fetch(manifest_url: &str, limits: Limits) -> Result<(Manifest, String)> {
    let url = reqwest::Url::parse(manifest_url.trim()).map_err(|e| Error::Extension(format!("not a valid URL: {e}")))?;
    let text = download(&url, MAX_MANIFEST_BYTES, limits).await?;
    let raw: Value = serde_json::from_str(&text).map_err(|e| Error::Extension(format!("the manifest isn't valid JSON: {e}")))?;

    let (manifest, code) = if convert::is_manifest(&raw) {
        convert::convert(&raw, &url, limits).await?
    } else {
        let mut manifest: Manifest =
            serde_json::from_value(raw).map_err(|e| Error::Extension(format!("the manifest isn't valid: {e}")))?;
        manifest.origin = None;
        if manifest.api_version != API_VERSION {
            return Err(Error::Extension(format!(
                "made for extension API v{}, this version of B-Anime supports v{API_VERSION}",
                manifest.api_version
            )));
        }
        let code_url = url
            .join(&manifest.code_url)
            .map_err(|e| Error::Extension(format!("bad codeUrl: {e}")))?;
        (manifest, download(&code_url, MAX_CODE_BYTES, limits).await?)
    };
    if !valid_id(&manifest.id) {
        return Err(Error::Extension(format!("invalid id {:?}: use lowercase letters, digits and dashes", manifest.id)));
    }

    // Load it in the sandbox and check it has what its kind needs.
    let probe = Limits { call_timeout: std::time::Duration::from_secs(10), ..limits };
    let methods: Vec<String> = serde_json::from_value(runtime::call(&code, "__methods", &Value::Array(vec![]), probe).await?.value)
        .map_err(|e| Error::Extension(e.to_string()))?;
    let missing: Vec<_> = manifest.kind.required_methods().iter().filter(|m| !methods.iter().any(|x| x == *m)).collect();
    if !missing.is_empty() {
        let list = missing.iter().map(|m| format!("{m}()")).collect::<Vec<_>>().join(", ");
        return Err(Error::Extension(format!("the Provider is missing {list}")));
    }
    Ok((manifest, code))
}

fn code_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.js"))
}

/// Installs (or replaces) an extension from its manifest URL.
pub async fn install(db: &Db, dir: &Path, manifest_url: &str, limits: Limits) -> Result<Installed> {
    let (manifest, code) = fetch(manifest_url, limits).await?;
    tokio::fs::create_dir_all(dir).await?;
    // Write then rename, so a half-written script is never loaded.
    let tmp = dir.join(format!("{}.js.part", manifest.id));
    tokio::fs::write(&tmp, &code).await?;
    tokio::fs::rename(&tmp, code_path(dir, &manifest.id)).await?;

    let json = serde_json::to_string(&manifest).map_err(|e| Error::Extension(e.to_string()))?;
    sqlx::query(
        "INSERT INTO extension (id, manifest, source_url) VALUES (?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET manifest = excluded.manifest, source_url = excluded.source_url,
           updated_at = datetime('now')",
    )
    .bind(&manifest.id)
    .bind(json)
    .bind(manifest_url.trim())
    .execute(db)
    .await?;
    get(db, &manifest.id).await?.ok_or_else(|| Error::Extension("install didn't stick".into()))
}

#[derive(sqlx::FromRow)]
struct Row {
    manifest: String,
    source_url: String,
    enabled: bool,
    installed_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl Row {
    fn into_installed(self) -> Option<Installed> {
        Some(Installed {
            manifest: serde_json::from_str(&self.manifest).ok()?,
            source_url: self.source_url,
            enabled: self.enabled,
            installed_at: self.installed_at,
            updated_at: self.updated_at,
        })
    }
}

const SELECT: &str = "SELECT manifest, source_url, enabled, installed_at, updated_at FROM extension";

pub async fn list(db: &Db) -> Result<Vec<Installed>> {
    let rows: Vec<Row> = sqlx::query_as(SELECT).fetch_all(db).await?;
    let mut list: Vec<_> = rows.into_iter().filter_map(Row::into_installed).collect();
    list.sort_by(|a, b| a.manifest.name.to_lowercase().cmp(&b.manifest.name.to_lowercase()));
    Ok(list)
}

pub async fn get(db: &Db, id: &str) -> Result<Option<Installed>> {
    let row: Option<Row> = sqlx::query_as("SELECT manifest, source_url, enabled, installed_at, updated_at FROM extension WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(row.and_then(Row::into_installed))
}

pub async fn set_enabled(db: &Db, id: &str, enabled: bool) -> Result<()> {
    sqlx::query("UPDATE extension SET enabled = ? WHERE id = ?").bind(enabled).bind(id).execute(db).await?;
    Ok(())
}

pub async fn remove(db: &Db, dir: &Path, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM extension WHERE id = ?").bind(id).execute(db).await?;
    match tokio::fs::remove_file(code_path(dir, id)).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// The version available at the extension's source, when it's newer than the installed one.
pub async fn available_update(ext: &Installed, limits: Limits) -> Result<Option<String>> {
    let url = reqwest::Url::parse(&ext.source_url).map_err(|e| Error::Extension(e.to_string()))?;
    let text = download(&url, MAX_MANIFEST_BYTES, limits).await?;
    // Both manifest formats carry a top-level "version".
    let latest: Value = serde_json::from_str(&text).map_err(|e| Error::Extension(e.to_string()))?;
    let version = latest.get("version").and_then(Value::as_str).ok_or_else(|| Error::Extension("the manifest has no version".into()))?;
    Ok(newer(version, &ext.manifest.version).then(|| version.to_owned()))
}

/// Runs a method of an installed, enabled extension.
pub async fn call(db: &Db, dir: &Path, id: &str, method: &str, args: &Value, limits: Limits) -> Result<CallOutput> {
    let ext = get(db, id).await?.ok_or_else(|| Error::Extension(format!("{id} isn't installed")))?;
    if !ext.enabled {
        return Err(Error::Extension(format!("{} is turned off", ext.manifest.name)));
    }
    let code = tokio::fs::read_to_string(code_path(dir, id)).await?;
    runtime::call(&code, method, args, limits).await
}

/// The repository B-Anime starts with: its own examples (public test videos and placeholder
/// pages). Users can remove it and add others.
pub const DEFAULT_REPO: &str = "https://raw.githubusercontent.com/Bas1874/Seanime-Marketplace/refs/heads/main/Marketplace/Main.json";

/// One extension listed in a repository.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoEntry {
    pub manifest_url: String,
    pub id: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub kind: Option<ExtKind>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub language: Option<String>,
    /// "converted" when the listing is in the single-manifest format (it converts on install).
    pub origin: Option<String>,
}

/// A repository: `{ "name", "description", "extensions": [...] }`, or just the array. Each item
/// is a manifest URL or an object with a `manifestUrl` plus listing details.
///
/// Single-manifest repositories work too: a marketplace (an array of manifests with `manifestURI`)
/// and `{ "urls": [...] }`. Types B-Anime can't run (torrent providers, plugins) are left
/// out of the listing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub url: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub extensions: Vec<RepoEntry>,
}

pub async fn read_repo(url: &str, limits: Limits) -> Result<Repo> {
    let base = reqwest::Url::parse(url.trim()).map_err(|e| Error::Extension(format!("not a valid URL: {e}")))?;
    let text = download(&base, 1024 * 1024, limits).await?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| Error::Extension(format!("not a repository: {e}")))?;
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let (name, description, items, url_list) = match &doc {
        Value::Array(items) => (None, None, items.clone(), false),
        Value::Object(_) if doc.get("urls").is_some_and(Value::is_array) => (
            s(&doc, "name"),
            s(&doc, "description"),
            doc["urls"].as_array().cloned().unwrap_or_default(),
            true,
        ),
        Value::Object(_) => (
            s(&doc, "name"),
            s(&doc, "description"),
            doc.get("extensions").and_then(Value::as_array).cloned().unwrap_or_default(),
            false,
        ),
        _ => return Err(Error::Extension("not a repository: expected a list or an object".into())),
    };
    let absolute = |raw: &str| base.join(raw).ok().map(|u| u.to_string());
    let extensions = items
        .into_iter()
        .filter_map(|v| {
            let raw = v.as_str().map(str::to_owned).or_else(|| s(&v, "manifestUrl")).or_else(|| s(&v, "manifestURI"))?;
            let manifest_url = absolute(&raw)?;
            // Those entries name their type; its `language` is the programming language and
            // `lang` the content's.
            let foreign_type = s(&v, "type");
            let (kind, language) = match &foreign_type {
                Some(t) => (Some(convert::kind(t).ok()?), Some(s(&v, "lang").filter(|l| !l.is_empty()).unwrap_or_else(|| "en".into()))),
                None => (v.get("kind").and_then(|k| serde_json::from_value(k.clone()).ok()), s(&v, "language")),
            };
            Some(RepoEntry {
                id: s(&v, "id").map(|id| if foreign_type.is_some() { convert::convert_id(&id) } else { id }),
                name: s(&v, "name").unwrap_or_else(|| manifest_url.clone()),
                version: s(&v, "version"),
                kind,
                author: s(&v, "author"),
                description: s(&v, "description"),
                icon: s(&v, "icon").filter(|i| !i.is_empty()).and_then(|i| absolute(&i)),
                language,
                origin: (foreign_type.is_some() || url_list).then(|| convert::ORIGIN.to_owned()),
                manifest_url,
            })
        })
        .collect();
    Ok(Repo { url: base.to_string(), name, description, extensions })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get as route};

    #[tokio::test]
    async fn the_documented_example_works() {
        let code = include_str!("../../../../docs/examples/demo-streams/main.js");
        let manifest: Manifest =
            serde_json::from_str(include_str!("../../../../docs/examples/demo-streams/manifest.json")).unwrap();
        assert_eq!(manifest.kind, ExtKind::Anime);
        let limits = Limits::default();
        let found = runtime::call(code, "search", &serde_json::json!([{ "query": "anything" }]), limits).await.unwrap();
        let id = found.value[0]["id"].as_str().unwrap().to_owned();
        let eps = runtime::call(code, "episodes", &serde_json::json!([id]), limits).await.unwrap();
        let ep = eps.value[0]["id"].as_str().unwrap().to_owned();
        let src = runtime::call(code, "sources", &serde_json::json!([ep]), limits).await.unwrap();
        assert_eq!(src.value["sources"][0]["type"], "hls");

        let manga = include_str!("../../../../docs/examples/demo-manga/main.js");
        let chapters = runtime::call(manga, "chapters", &serde_json::json!(["demo"]), limits).await.unwrap();
        let first = chapters.value[0]["id"].as_str().unwrap().to_owned();
        let pages = runtime::call(manga, "pages", &serde_json::json!([first]), limits).await.unwrap();
        assert_eq!(pages.value["pages"].as_array().unwrap().len(), 8);
    }

    #[tokio::test]
    async fn the_starter_repository_lists_the_examples() {
        let base = serve(
            Router::new()
                .route("/docs/repository.json", route(|| async { include_str!("../../../../docs/repository.json") }))
                .route("/docs/examples/demo-streams/manifest.json", route(|| async {
                    include_str!("../../../../docs/examples/demo-streams/manifest.json")
                }))
                .route("/docs/examples/demo-manga/manifest.json", route(|| async {
                    include_str!("../../../../docs/examples/demo-manga/manifest.json")
                })),
        )
        .await;
        let repo = read_repo(&format!("{base}/docs/repository.json"), test_limits()).await.unwrap();
        assert_eq!(repo.extensions.len(), 2);
        for entry in &repo.extensions {
            // The listing must agree with the manifest it points at.
            let manifest: Manifest = serde_json::from_str(&reqwest::get(&entry.manifest_url).await.unwrap().text().await.unwrap()).unwrap();
            assert_eq!(entry.id.as_deref(), Some(manifest.id.as_str()));
            assert_eq!(entry.version.as_deref(), Some(manifest.version.as_str()));
            assert_eq!(entry.kind, Some(manifest.kind));
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("1.10.0", "1.9.2"));
        assert!(newer("2", "1.99"));
        assert!(!newer("1.0.0", "1.0.0"));
        assert!(!newer("0.9", "1.0"));
    }

    #[test]
    fn ids_are_strict() {
        assert!(valid_id("my-source-2"));
        assert!(!valid_id("../evil"));
        assert!(!valid_id("Caps"));
        assert!(!valid_id("-x"));
    }

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn test_limits() -> Limits {
        Limits { allow_private_hosts: true, ..Limits::default() }
    }

    #[tokio::test]
    async fn install_call_update_remove() {
        const CODE: &str = r#"class Provider {
            async search(q) { return [{ id: "a", title: "Result for " + q.query }]; }
            async episodes(id) { return [{ id: id + "-1", number: 1 }]; }
            async sources(ep) { return { sources: [{ url: "https://example.com/" + ep + ".m3u8", type: "hls" }] }; }
        }"#;
        let base = serve(
            Router::new()
                .route("/m.json", route(|| async {
                    r#"{"id":"test-source","name":"Test","version":"1.0.0","kind":"anime","codeUrl":"main.js","apiVersion":1}"#
                }))
                .route("/main.js", route(|| async { CODE }))
                .route("/repo.json", route(|| async { r#"["m.json", {"manifestUrl":"other.json","name":"Other"}]"# }))
                .route("/named.json", route(|| async {
                    r#"{"name":"Named repo","extensions":[{"manifestUrl":"m.json","name":"Test","kind":"anime","icon":"icon.png"}]}"#
                }))
                .route("/broken.json", route(|| async {
                    r#"{"id":"broken","name":"Broken","version":"1","kind":"manga","codeUrl":"main.js","apiVersion":1}"#
                })),
        )
        .await;

        let db = crate::db::open_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("banime-ext-test-{}", std::process::id()));

        let ext = install(&db, &dir, &format!("{base}/m.json"), test_limits()).await.unwrap();
        assert_eq!((ext.manifest.id.as_str(), ext.enabled), ("test-source", true));

        let out = call(&db, &dir, "test-source", "search", &serde_json::json!([{ "query": "x" }]), test_limits()).await.unwrap();
        assert_eq!(out.value[0]["title"], "Result for x");

        set_enabled(&db, "test-source", false).await.unwrap();
        assert!(call(&db, &dir, "test-source", "search", &serde_json::json!([{}]), test_limits()).await.is_err());

        assert_eq!(available_update(&ext, test_limits()).await.unwrap(), None);

        // A manga manifest pointing at an anime provider is refused.
        let err = install(&db, &dir, &format!("{base}/broken.json"), test_limits()).await.unwrap_err();
        assert!(err.to_string().contains("missing chapters(), pages()"), "{err}");

        let repo = read_repo(&format!("{base}/repo.json"), test_limits()).await.unwrap();
        assert_eq!(repo.extensions.len(), 2);
        assert_eq!(repo.extensions[1].manifest_url, format!("{base}/other.json"));
        let named = read_repo(&format!("{base}/named.json"), test_limits()).await.unwrap();
        assert_eq!(named.name.as_deref(), Some("Named repo"));
        assert_eq!(named.extensions[0].icon.as_deref(), Some(format!("{base}/icon.png").as_str()));
        assert_eq!(named.extensions[0].kind, Some(ExtKind::Anime));

        remove(&db, &dir, "test-source").await.unwrap();
        assert!(list(&db).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn converted_repositories_and_manifests_install() {
        const TS: &str = r#"class Provider {
            getSettings(): Settings { return { episodeServers: ["one"], supportsDub: false } }
            async search(opts: SearchOptions): Promise<SearchResult[]> { return [{ id: "s", title: opts.query, url: "", subOrDub: "sub" }] }
            async findEpisodes(id: string): Promise<EpisodeDetails[]> { return [{ id: "e1", number: 1, url: "" }] }
            async findEpisodeServer(ep: EpisodeDetails, server: string): Promise<EpisodeServer> {
                return { server, headers: {}, videoSources: [{ url: "https://example.com/" + ep.id + ".mp4", type: "mp4", quality: "720p", subtitles: [] }] }
            }
        }"#;
        let base = serve(
            Router::new()
                .route("/inline.json", route(|| async {
                    serde_json::json!({
                        "id": "My_Stream", "name": "My stream", "version": "2.1.0", "type": "onlinestream-provider",
                        "language": "typescript", "lang": "en", "author": "someone", "manifestURI": "inline.json", "payload": TS,
                    }).to_string()
                }))
                .route("/remote.json", route(|| async {
                    r#"{"id":"remote-manga","name":"Remote manga","version":"1.0.0","type":"manga-provider","language":"javascript","payloadURI":"remote.js"}"#
                }))
                .route("/remote.js", route(|| async {
                    "class Provider { async search(){return []} async findChapters(){return []} async findChapterPages(){return []} getSettings(){return {}} }"
                }))
                .route("/marketplace.json", route(|| async {
                    r#"[
                        {"id":"My_Stream","name":"My stream","version":"2.1.0","type":"onlinestream-provider","language":"typescript","lang":"","manifestURI":"inline.json"},
                        {"id":"torrents","name":"Torrents","version":"1","type":"anime-torrent-provider","language":"go","manifestURI":"t.json"},
                        {"id":"remote-manga","name":"Remote manga","version":"1.0.0","type":"manga-provider","language":"javascript","lang":"multi","manifestURI":"remote.json"}
                    ]"#
                }))
                .route("/urls.json", route(|| async { r#"{"urls":["inline.json","remote.json"]}"# })),
        )
        .await;

        let market = read_repo(&format!("{base}/marketplace.json"), test_limits()).await.unwrap();
        assert_eq!(market.extensions.len(), 2, "torrent providers are left out");
        assert_eq!(market.extensions[0].id.as_deref(), Some("my-stream"));
        assert_eq!(market.extensions[0].kind, Some(ExtKind::Anime));
        assert_eq!(market.extensions[0].language.as_deref(), Some("en"));
        assert_eq!(market.extensions[1].language.as_deref(), Some("multi"));
        assert_eq!(market.extensions[1].origin.as_deref(), Some("converted"));
        let urls = read_repo(&format!("{base}/urls.json"), test_limits()).await.unwrap();
        assert_eq!(urls.extensions.len(), 2);
        assert_eq!(urls.extensions[1].manifest_url, format!("{base}/remote.json"));

        let db = crate::db::open_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("banime-converted-test-{}", std::process::id()));
        let ext = install(&db, &dir, &format!("{base}/inline.json"), test_limits()).await.unwrap();
        assert_eq!((ext.manifest.id.as_str(), ext.manifest.version.as_str()), ("my-stream", "2.1.0"));
        assert_eq!(ext.manifest.origin.as_deref(), Some("converted"));
        let eps = call(&db, &dir, "my-stream", "episodes", &serde_json::json!(["s"]), test_limits()).await.unwrap().value;
        let src = call(&db, &dir, "my-stream", "sources", &serde_json::json!([eps[0]["id"]]), test_limits()).await.unwrap().value;
        assert_eq!(src["sources"][0], serde_json::json!({ "url": "https://example.com/e1.mp4", "type": "mp4", "label": "720p" }));
        assert_eq!(available_update(&ext, test_limits()).await.unwrap(), None);

        let manga = install(&db, &dir, &format!("{base}/remote.json"), test_limits()).await.unwrap();
        assert_eq!(manga.manifest.kind, ExtKind::Manga);
        assert_eq!(manga.manifest.code_url, format!("{base}/remote.js"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
