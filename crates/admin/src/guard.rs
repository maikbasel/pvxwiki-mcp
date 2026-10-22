//! Request checks for the admin listener: Host allow-list and cross-site POST refusal.

use axum::http::HeaderMap;

/// Hosts the admin page always answers to, besides `PVXWIKI_ALLOWED_HOSTS`.
const LOCAL_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// True when `host` (a `Host` header, port optional) is local or in `allowed`.
pub(crate) fn host_allowed(host: &str, allowed: &[String]) -> bool {
    let name = match host.strip_prefix('[') {
        // `[::1]:8081`: an IPv6 literal keeps its colons.
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    !name.is_empty()
        && (LOCAL_HOSTS.contains(&name) || allowed.iter().any(|a| a.eq_ignore_ascii_case(name)))
}

/// True for a request a cross-site page cannot forge: htmx's `HX-Request: true` (a custom
/// header, which a cross-site form cannot set) and a same-origin or absent `Sec-Fetch-Site`.
/// The browser sends basic-auth credentials on its own, so this is the CSRF guard.
pub(crate) fn same_origin_htmx(headers: &HeaderMap) -> bool {
    let is = |name: &str, value: &str| headers.get(name).is_some_and(|v| v == value);
    is("hx-request", "true")
        && (headers.get("sec-fetch-site").is_none() || is("sec-fetch-site", "same-origin"))
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderName, HeaderValue};

    use super::*;

    fn allowed() -> Vec<String> {
        vec!["admin.example.org".into()]
    }

    #[test]
    fn local_and_listed_hosts_are_allowed_with_or_without_a_port() {
        assert!(host_allowed("localhost:8081", &allowed()));
        assert!(host_allowed("127.0.0.1", &allowed()));
        assert!(host_allowed("[::1]:8081", &allowed()));
        assert!(host_allowed("admin.example.org:8443", &allowed()));
        assert!(host_allowed("Admin.Example.org", &allowed()));
    }

    #[test]
    fn other_hosts_are_refused() {
        assert!(!host_allowed("evil.example", &allowed()));
        assert!(!host_allowed("admin.example.org.evil.example", &allowed()));
        assert!(!host_allowed("", &allowed()));
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(k, v)| (HeaderName::from_static(k), HeaderValue::from_static(v)))
            .collect()
    }

    #[test]
    fn same_origin_htmx_requests_pass() {
        assert!(same_origin_htmx(&headers(&[
            ("hx-request", "true"),
            ("sec-fetch-site", "same-origin")
        ])));
        // Non-browser clients send no Sec-Fetch-Site.
        assert!(same_origin_htmx(&headers(&[("hx-request", "true")])));
    }

    #[test]
    fn cross_site_or_non_htmx_requests_are_refused() {
        assert!(!same_origin_htmx(&headers(&[
            ("hx-request", "true"),
            ("sec-fetch-site", "cross-site")
        ])));
        assert!(!same_origin_htmx(&headers(&[
            ("hx-request", "true"),
            ("sec-fetch-site", "same-site")
        ])));
        assert!(!same_origin_htmx(&headers(&[(
            "sec-fetch-site",
            "same-origin"
        )])));
        assert!(!same_origin_htmx(&headers(&[("hx-request", "false")])));
    }
}
