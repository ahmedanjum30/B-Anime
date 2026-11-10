//! Helpers the sandbox exposes to extension code beyond `fetch`: an HTML parser with CSS
//! selectors, a few crypto primitives and URL parsing. All of them are pure computation on data
//! the extension already has; none reach the network or the disk.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use ego_tree::{NodeId, NodeRef};
use scraper::{ElementRef, Html, Node, Selector};
use serde_json::{Map, Value, json};

/// Parsed documents for one call. Nodes are handed to JS as small integers.
#[derive(Default)]
pub struct Docs {
    docs: Vec<Doc>,
}

struct Doc {
    html: Html,
    ids: Vec<NodeId>,
    index: HashMap<NodeId, u32>,
}

const MAX_DOCS: usize = 64;

impl Doc {
    fn intern(&mut self, id: NodeId) -> u32 {
        if let Some(n) = self.index.get(&id) {
            return *n;
        }
        let n = self.ids.len() as u32;
        self.ids.push(id);
        self.index.insert(id, n);
        n
    }

    fn node(&self, n: u32) -> Option<NodeRef<'_, Node>> {
        self.ids.get(n as usize).and_then(|id| self.html.tree.get(*id))
    }

    fn text_of(node: NodeRef<'_, Node>, out: &mut String) {
        for d in node.descendants() {
            if let Node::Text(t) = d.value() {
                out.push_str(t);
            }
        }
    }
}

impl Docs {
    /// Parses a document; its root node is id 0.
    pub fn parse(&mut self, html: &str) -> Result<u32, String> {
        if self.docs.len() >= MAX_DOCS {
            return Err(format!("more than {MAX_DOCS} documents parsed in one call"));
        }
        let html = Html::parse_document(html);
        let mut doc = Doc { ids: Vec::new(), index: HashMap::new(), html };
        let root = doc.html.tree.root().id();
        doc.intern(root);
        self.docs.push(doc);
        Ok(self.docs.len() as u32 - 1)
    }

    /// One selection operation. Navigation returns node ids (document order within each input
    /// node, no duplicates); `text`, `html`, `outerHtml`, `attrs` and `name` return values.
    pub fn op(&mut self, doc: u32, op: &str, ids: &[u32], arg: &str) -> Result<Value, String> {
        let d = self.docs.get_mut(doc as usize).ok_or("unknown document")?;
        let selector = || Selector::parse(arg).map_err(|e| format!("bad selector {arg:?}: {e}"));
        let nodes: Vec<NodeId> = ids.iter().filter_map(|n| d.node(*n).map(|x| x.id())).collect();
        let mut found: Vec<NodeId> = Vec::new();
        let mut seen = HashSet::new();
        let mut push = |id: NodeId, found: &mut Vec<NodeId>| {
            if seen.insert(id) {
                found.push(id);
            }
        };

        match op {
            "find" => {
                let sel = selector()?;
                for id in &nodes {
                    let node = d.html.tree.get(*id).unwrap();
                    for e in node.descendants().skip(1).filter_map(ElementRef::wrap) {
                        if sel.matches(&e) {
                            push(e.id(), &mut found);
                        }
                    }
                }
            }
            "filter" => {
                let sel = selector()?;
                for id in &nodes {
                    if let Some(e) = ElementRef::wrap(d.html.tree.get(*id).unwrap()) {
                        if sel.matches(&e) {
                            push(e.id(), &mut found);
                        }
                    }
                }
            }
            "children" | "contents" => {
                let all = op == "contents";
                for id in &nodes {
                    for c in d.html.tree.get(*id).unwrap().children() {
                        if all || c.value().is_element() {
                            push(c.id(), &mut found);
                        }
                    }
                }
            }
            "parent" => {
                for id in &nodes {
                    if let Some(p) = d.html.tree.get(*id).unwrap().parent() {
                        if p.value().is_element() {
                            push(p.id(), &mut found);
                        }
                    }
                }
            }
            "next" | "prev" => {
                for id in &nodes {
                    let mut cur = d.html.tree.get(*id).unwrap();
                    loop {
                        let sib = if op == "next" { cur.next_sibling() } else { cur.prev_sibling() };
                        match sib {
                            Some(s) if s.value().is_element() => {
                                push(s.id(), &mut found);
                                break;
                            }
                            Some(s) => cur = s,
                            None => break,
                        }
                    }
                }
            }
            "text" => {
                let mut out = String::new();
                for id in &nodes {
                    Doc::text_of(d.html.tree.get(*id).unwrap(), &mut out);
                }
                return Ok(Value::String(out));
            }
            "html" | "outerHtml" => {
                let Some(id) = nodes.first() else { return Ok(Value::Null) };
                let node = d.html.tree.get(*id).unwrap();
                return Ok(match (ElementRef::wrap(node), node.value()) {
                    (Some(e), _) => Value::String(if op == "html" { e.inner_html() } else { e.html() }),
                    (None, Node::Document) => Value::String(d.html.html()),
                    (None, Node::Text(t)) => Value::String(t.to_string()),
                    _ => Value::Null,
                });
            }
            "attrs" => {
                let Some(e) = nodes.first().and_then(|id| ElementRef::wrap(d.html.tree.get(*id).unwrap())) else {
                    return Ok(Value::Null);
                };
                let map: Map<String, Value> = e.value().attrs().map(|(k, v)| (k.to_owned(), Value::from(v))).collect();
                return Ok(Value::Object(map));
            }
            "name" => {
                let name = nodes.first().and_then(|id| ElementRef::wrap(d.html.tree.get(*id).unwrap())).map(|e| e.value().name().to_owned());
                return Ok(name.map(Value::String).unwrap_or(Value::Null));
            }
            other => return Err(format!("unknown document operation {other}")),
        }
        Ok(Value::Array(found.into_iter().map(|id| Value::from(d.intern(id))).collect()))
    }
}

fn unb64(s: &str) -> Result<Vec<u8>, String> {
    B64.decode(s).map_err(|e| format!("bad base64: {e}"))
}

/// Crypto primitives over base64 strings: AES-CBC with PKCS#7 padding (key size picks
/// AES-128/192/256), MD5 and SHA-256.
pub fn crypto(op: &str, a: &str, b: &str, c: &str) -> Result<String, String> {
    use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
    use sha2::Digest;

    let data = unb64(a)?;
    let out = match op {
        "md5" => md5::Md5::digest(&data).to_vec(),
        "sha256" => sha2::Sha256::digest(&data).to_vec(),
        "aes-cbc-encrypt" | "aes-cbc-decrypt" => {
            let (key, iv) = (unb64(b)?, unb64(c)?);
            if iv.len() != 16 {
                return Err("the IV must be 16 bytes".into());
            }
            let encrypt = op == "aes-cbc-encrypt";
            macro_rules! run {
                ($aes:ty) => {{
                    if encrypt {
                        let mut buf = data.clone();
                        buf.resize(data.len() + 16, 0);
                        cbc::Encryptor::<$aes>::new_from_slices(&key, &iv)
                            .map_err(|e| e.to_string())?
                            .encrypt_padded_mut::<Pkcs7>(&mut buf, data.len())
                            .map_err(|e| e.to_string())?
                            .to_vec()
                    } else {
                        let mut buf = data.clone();
                        cbc::Decryptor::<$aes>::new_from_slices(&key, &iv)
                            .map_err(|e| e.to_string())?
                            .decrypt_padded_mut::<Pkcs7>(&mut buf)
                            .map_err(|_| "decryption failed (wrong key or IV)".to_string())?
                            .to_vec()
                    }
                }};
            }
            match key.len() {
                16 => run!(aes::Aes128),
                24 => run!(aes::Aes192),
                32 => run!(aes::Aes256),
                n => return Err(format!("AES keys are 16, 24 or 32 bytes, not {n}")),
            }
        }
        other => return Err(format!("unknown crypto operation {other}")),
    };
    Ok(B64.encode(out))
}

/// WHATWG-style URL parts, the shape JS `URL` exposes.
pub fn parse_url(input: &str, base: &str) -> Result<Value, String> {
    let url = if base.is_empty() {
        reqwest::Url::parse(input)
    } else {
        let base = reqwest::Url::parse(base).map_err(|e| format!("Invalid base URL {base:?}: {e}"))?;
        base.join(input)
    }
    .map_err(|e| format!("Invalid URL {input:?}: {e}"))?;
    let port = url.port().map(|p| p.to_string()).unwrap_or_default();
    let hostname = url.host_str().unwrap_or_default().to_owned();
    let host = if port.is_empty() { hostname.clone() } else { format!("{hostname}:{port}") };
    Ok(json!({
        "href": url.as_str(),
        "protocol": format!("{}:", url.scheme()),
        "username": url.username(),
        "password": url.password().unwrap_or_default(),
        "host": host,
        "hostname": hostname,
        "port": port,
        "pathname": url.path(),
        "search": url.query().filter(|q| !q.is_empty()).map(|q| format!("?{q}")).unwrap_or_default(),
        "hash": url.fragment().filter(|f| !f.is_empty()).map(|f| format!("#{f}")).unwrap_or_default(),
        "origin": url.origin().ascii_serialization(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_and_reads() {
        let mut docs = Docs::default();
        let d = docs.parse(r#"<ul><li class="a" data-x="1"><a href="/one">One</a></li><li>Two <b>2</b></li></ul>"#).unwrap();
        let lis = docs.op(d, "find", &[0], "li").unwrap();
        let lis: Vec<u32> = serde_json::from_value(lis).unwrap();
        assert_eq!(lis.len(), 2);
        assert_eq!(docs.op(d, "text", &lis, "").unwrap(), "OneTwo 2");
        let a: Vec<u32> = serde_json::from_value(docs.op(d, "find", &lis[..1], "a").unwrap()).unwrap();
        assert_eq!(docs.op(d, "attrs", &a, "").unwrap(), json!({ "href": "/one" }));
        assert_eq!(docs.op(d, "html", &lis[1..], "").unwrap(), "Two <b>2</b>");
        let filtered: Vec<u32> = serde_json::from_value(docs.op(d, "filter", &lis, ".a").unwrap()).unwrap();
        assert_eq!(filtered, lis[..1]);
        let next: Vec<u32> = serde_json::from_value(docs.op(d, "next", &lis[..1], "").unwrap()).unwrap();
        assert_eq!(next, lis[1..]);
        let parent: Vec<u32> = serde_json::from_value(docs.op(d, "parent", &lis, "").unwrap()).unwrap();
        assert_eq!(docs.op(d, "name", &parent, "").unwrap(), "ul");
        assert!(docs.op(d, "find", &[0], "li[").is_err());
    }

    #[test]
    fn aes_round_trips_and_matches_known_vectors() {
        let key = B64.encode([7u8; 32]);
        let iv = B64.encode([1u8; 16]);
        let enc = crypto("aes-cbc-encrypt", &B64.encode(b"hello world"), &key, &iv).unwrap();
        let dec = crypto("aes-cbc-decrypt", &enc, &key, &iv).unwrap();
        assert_eq!(B64.decode(dec).unwrap(), b"hello world");
        assert!(crypto("aes-cbc-decrypt", &enc, &B64.encode([8u8; 32]), &iv).is_err());
        // md5("abc")
        let md5 = crypto("md5", &B64.encode(b"abc"), "", "").unwrap();
        assert_eq!(B64.decode(md5).unwrap(), hex("900150983cd24fb0d6963f7d28e17f72"));
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn urls_parse_like_the_web() {
        let u = parse_url("../b?x=1#top", "https://example.com:8443/a/c").unwrap();
        assert_eq!(u["href"], "https://example.com:8443/b?x=1#top");
        assert_eq!(u["host"], "example.com:8443");
        assert_eq!(u["search"], "?x=1");
        assert_eq!(u["hash"], "#top");
        assert!(parse_url("not a url", "").is_err());
    }
}
