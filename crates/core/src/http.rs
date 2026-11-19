//! HTTP clients that trust the right certificates on every platform.
//!
//! Desktop: reqwest's default, the OS certificate store (rustls-platform-verifier).
//! Android: that verifier needs JNI setup and extra Kotlin classes in the APK, and without them
//! every HTTPS request fails. Android builds trust Mozilla's root list (webpki-root-certs) instead.

/// Start every reqwest client here instead of `reqwest::Client::builder()`.
pub fn client_builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(android_roots());
    builder
}

#[cfg(target_os = "android")]
fn android_roots() -> Vec<reqwest::Certificate> {
    webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|der| reqwest::Certificate::from_der(der).ok())
        .collect()
}
