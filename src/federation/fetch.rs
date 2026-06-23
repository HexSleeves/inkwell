//! The single guarded HTTP fetch every federation outbound request goes through.
//!
//! There is exactly one way for federation code to touch the network — both
//! Webmention source verification (receive) and Webmention delivery (send) call
//! [`guarded_get`] / [`guarded_post`]. Each request:
//!   1. parses + scheme-checks the URL ([`super::ssrf::validate_public_url`]);
//!   2. resolves the host's IPs and rejects the request if ANY resolved address
//!      is in a blocked range ([`super::ssrf::is_blocked_ip`]) — fail closed;
//!   3. pins the connection to the validated IPs (`resolve_to_addrs`) so reqwest
//!      connects to exactly what we checked, closing the DNS-rebinding gap;
//!   4. disables reqwest's automatic redirects and follows them MANUALLY, up to
//!      a small cap, re-running the full validation on every hop so a redirect
//!      can never bounce the fetch into a private range;
//!   5. caps the response body size and the total timeout.
//!
//! A blocked or failed fetch returns an error that the caller logs and drops —
//! it never panics and never propagates as a 500.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::Url;
use reqwest::redirect::Policy;
use tokio::time::Instant;

use super::ssrf::{is_blocked_ip, validate_public_url};

/// Per-request total timeout. Short so a slow or hostile endpoint can't tie up a
/// worker — federation fetches are best-effort and must never block for long.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum response body we will read. A Webmention source page only needs to be
/// scanned for a link to the target and an endpoint page only for a `<link>`, so
/// a small cap is plenty and bounds memory against a hostile large response.
pub const MAX_RESPONSE_BYTES: usize = 512 * 1024;

/// Maximum redirect hops we follow manually. Each hop is fully re-validated, so
/// this only bounds how many legitimate redirects a source may use.
const MAX_REDIRECTS: usize = 5;

/// User-Agent sent on every federation fetch so receivers can identify us.
const USER_AGENT: &str = concat!("inkwell-webmention/", env!("CARGO_PKG_VERSION"));

/// A fetched response: final URL (after any followed redirects) and the body,
/// already truncated to [`MAX_RESPONSE_BYTES`].
#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub final_url: Url,
    pub body: String,
}

/// SSRF-guarded `GET`. Follows redirects manually, re-validating each hop.
pub async fn guarded_get(url: &str) -> Result<FetchedPage> {
    guarded_request(reqwest::Method::GET, url, None).await
}

/// SSRF-guarded `POST` with a form-encoded body. Used by Webmention send to
/// deliver `source`/`target` to a discovered endpoint. Redirects are followed
/// manually and re-validated, just like `GET`.
pub async fn guarded_post(url: &str, form: &[(&str, &str)]) -> Result<FetchedPage> {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form.iter().copied())
        .finish();
    guarded_request(reqwest::Method::POST, url, Some(encoded)).await
}

/// Resolve a host:port to socket addresses, returning only the addresses that
/// pass the SSRF deny-list. Empty result ⇒ the host is unresolvable or every
/// address it resolves to is blocked.
async fn resolve_allowed_addrs(host: &str, port: u16) -> Result<Vec<SocketAddr>> {
    // `lookup_host` performs the actual DNS resolution off the URL's host.
    let resolved = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("resolving host {host}"))?;
    let mut allowed = Vec::new();
    for addr in resolved {
        if is_blocked_ip(&addr.ip()) {
            // Fail closed: one blocked address for this host aborts the fetch,
            // we do NOT cherry-pick a "safe" address from a host that also
            // resolves to an internal one.
            bail!("host {host} resolves to a blocked address ({})", addr.ip());
        }
        allowed.push(addr);
    }
    if allowed.is_empty() {
        bail!("host {host} did not resolve to any address");
    }
    Ok(allowed)
}

/// The shared request engine. Validates the URL, resolves + checks the host,
/// pins reqwest to the validated IPs, disables auto-redirects, and follows
/// redirects manually with full re-validation per hop.
async fn guarded_request(
    method: reqwest::Method,
    url: &str,
    form_body: Option<String>,
) -> Result<FetchedPage> {
    let mut current = validate_public_url(url).context("URL failed SSRF validation")?;
    // One end-to-end deadline across ALL redirect hops. A fresh client is built
    // per hop (to re-pin `resolve_to_addrs`), so without this a chain of
    // redirects could each get a full `FETCH_TIMEOUT` and run far longer than
    // intended; the remaining budget is applied as each hop's client timeout.
    let deadline = Instant::now() + FETCH_TIMEOUT;

    for _hop in 0..=MAX_REDIRECTS {
        let now = Instant::now();
        if now >= deadline {
            bail!("federation fetch timed out after {FETCH_TIMEOUT:?}");
        }
        let remaining = deadline.saturating_duration_since(now);

        // The host is guaranteed present by `validate_public_url`.
        let host = current
            .host_str()
            .context("validated URL unexpectedly had no host")?
            .to_string();
        let port = current
            .port_or_known_default()
            .context("URL has no port and an unknown default")?;

        // Resolve + deny-list check, then pin reqwest to exactly these IPs so it
        // cannot re-resolve to a different (internal) address between our check
        // and its connect (DNS-rebinding defense). `.no_proxy()` disables
        // automatic env/system proxy detection — with a proxy active the proxy
        // becomes the connection destination and `resolve_to_addrs` is no longer
        // authoritative, which would defeat the IP-pinning SSRF defense.
        let addrs = resolve_allowed_addrs(&host, port).await?;
        let client = reqwest::Client::builder()
            .timeout(remaining)
            .no_proxy()
            .redirect(Policy::none())
            .user_agent(USER_AGENT)
            .resolve_to_addrs(&host, &addrs)
            .build()
            .context("building guarded HTTP client")?;

        let mut request = client.request(method.clone(), current.clone());
        if let Some(body) = &form_body {
            request = request
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(body.clone());
        }
        let response = request.send().await.context("federation fetch failed")?;
        let status = response.status();

        // Manual redirect handling: resolve the Location against the current URL
        // and loop so the next hop is re-validated from the top.
        if status.is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .context("redirect response without a usable Location header")?;
            let next = current
                .join(location)
                .context("redirect Location is not a resolvable URL")?;
            // Re-apply the syntactic guard (scheme allowlist + literal-IP check);
            // the loop's next iteration re-resolves and re-checks the new host.
            current = validate_public_url(next.as_str())
                .context("redirect target failed SSRF validation")?;
            continue;
        }

        if !status.is_success() {
            bail!("federation fetch returned status {status}");
        }

        let body = read_capped_body(response).await?;
        return Ok(FetchedPage {
            final_url: current,
            body,
        });
    }

    bail!("federation fetch exceeded {MAX_REDIRECTS} redirects")
}

/// Read a response body, stopping once [`MAX_RESPONSE_BYTES`] have been
/// collected so a hostile endpoint can't exhaust memory. Bytes are decoded
/// lossily as UTF-8 (HTML/text only needs a best-effort scan for a link).
async fn read_capped_body(response: reqwest::Response) -> Result<String> {
    use futures_util::StreamExt;

    let mut collected: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading federation response body")?;
        let remaining = MAX_RESPONSE_BYTES.saturating_sub(collected.len());
        if remaining == 0 {
            break;
        }
        let take = remaining.min(chunk.len());
        collected.extend_from_slice(&chunk[..take]);
        if take < chunk.len() {
            // Hit the cap mid-chunk; stop reading the rest of the body.
            break;
        }
    }
    Ok(String::from_utf8_lossy(&collected).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_disallowed_scheme_before_any_network() {
        let err = guarded_get("ftp://example.com/x").await.unwrap_err();
        assert!(err.to_string().contains("SSRF"));
    }

    #[tokio::test]
    async fn rejects_literal_loopback_without_network() {
        let err = guarded_get("http://127.0.0.1/x").await.unwrap_err();
        assert!(err.to_string().contains("SSRF"));
    }

    #[tokio::test]
    async fn rejects_literal_metadata_address() {
        let err = guarded_get("http://169.254.169.254/latest/")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("SSRF"));
    }

    #[tokio::test]
    async fn resolve_allowed_addrs_blocks_localhost() {
        // `localhost` resolves to loopback, which must be refused.
        let err = resolve_allowed_addrs("localhost", 80).await.unwrap_err();
        assert!(err.to_string().contains("blocked address"));
    }
}
