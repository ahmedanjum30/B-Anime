//! B-Anime's own repository (`docs/repository.json`) must agree with the manifests it lists.
//! Browse shows the listing's version, but installing and updating read the manifest: when the
//! two differ, users see an update that changes nothing.

use std::path::{Path, PathBuf};

use serde_json::Value;

fn docs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("can't read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} isn't valid JSON: {e}", path.display()))
}

#[test]
fn listing_matches_each_manifest() {
    let listing = read_json(&docs().join("repository.json"));
    let entries = listing["extensions"].as_array().expect("repository.json has an \"extensions\" array");

    let mut problems = Vec::new();
    for entry in entries {
        let url = entry["manifestUrl"].as_str().expect("every entry has a manifestUrl");
        let manifest = read_json(&docs().join(url));
        for field in ["id", "version", "name", "kind"] {
            let listed = &entry[field];
            if !listed.is_null() && *listed != manifest[field] {
                problems.push(format!("{url}: \"{field}\" is {listed} in repository.json but {} in the manifest", manifest[field]));
            }
        }
        if entry["recommended"].as_bool().unwrap_or(false) != manifest["recommended"].as_bool().unwrap_or(false) {
            problems.push(format!("{url}: \"recommended\" differs between repository.json and the manifest"));
        }
    }
    assert!(problems.is_empty(), "repository.json and the manifests disagree:\n  {}", problems.join("\n  "));
}
