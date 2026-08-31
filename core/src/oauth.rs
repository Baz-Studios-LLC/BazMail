//! OAuth 2.0 for Fastmail, as a public native client.
//!
//! Fastmail advertises everything needed to do this properly and without any
//! developer sign-up: dynamic client registration (RFC 7591), PKCE with S256,
//! `token_endpoint_auth_method: none`, and an `offline_access` scope for refresh
//! tokens. So BazMail registers itself the first time you connect and there is
//! no client secret to ship — which is the point, since a secret embedded in a
//! desktop app is not a secret.
//!
//! This replaces pasting a hand-made API token. The difference that matters is
//! not convenience: an API token is long-lived, unscoped past what you tick, and
//! invisible once created. An OAuth grant is scoped, listed in Fastmail's own
//! UI, revocable from their side, and the thing we store is a refresh token that
//! buys short-lived access tokens.
//!
//! The redirect is a loopback URI per RFC 8252 — never a custom scheme, which
//! any other app on the machine could claim.

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const FASTMAIL_ISSUER: &str = "https://api.fastmail.com";
pub const SCOPE_MAIL: &str = "urn:ietf:params:oauth:scope:mail";
pub const SCOPE_OFFLINE: &str = "offline_access";
/// The resource indicator (RFC 8707). Fastmail's protected-resource metadata
/// names this exact URL, and it *requires* the parameter: omitting it fails the
/// authorization with `invalid_target` rather than defaulting to anything.
pub const FASTMAIL_RESOURCE: &str = "https://api.fastmail.com/jmap/session";

#[derive(Debug, Clone, Deserialize)]
pub struct Endpoints {
    pub registration_endpoint: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub revocation_endpoint: Option<String>,
}

/// Reads the authorization server's own metadata rather than hard-coding URLs,
/// so a change on Fastmail's side does not silently break sign-in.
pub async fn discover(http: &reqwest::Client, issuer: &str) -> Result<Endpoints> {
    let url = format!("{}/.well-known/oauth-authorization-server", issuer.trim_end_matches('/'));
    http.get(&url)
        .send()
        .await
        .context("fetching OAuth metadata")?
        .error_for_status()
        .context("OAuth metadata request rejected")?
        .json()
        .await
        .context("parsing OAuth metadata")
}

/// A PKCE pair. The verifier stays in this process; only its hash crosses the
/// network, so an intercepted authorization code is useless without us.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn pkce() -> Pkce {
    let verifier = random_urlsafe(32);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    Pkce {
        verifier,
        challenge,
    }
}

pub fn random_urlsafe(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("system randomness unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

#[derive(Debug, Deserialize)]
struct Registration {
    client_id: String,
}

/// Registers BazMail as a public client. The returned id is not a secret and is
/// kept in config.json; it is stable, so this runs once per install.
pub async fn register(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    redirect_uri: &str,
) -> Result<String> {
    let body = serde_json::json!({
        "client_name": "BazMail",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "native",
        "scope": format!("{SCOPE_MAIL} {SCOPE_OFFLINE}"),
    });

    let registration: Registration = http
        .post(&endpoints.registration_endpoint)
        .json(&body)
        .send()
        .await
        .context("registering the OAuth client")?
        .error_for_status()
        .context("client registration rejected")?
        .json()
        .await
        .context("parsing client registration")?;

    Ok(registration.client_id)
}

pub fn authorize_url(
    endpoints: &Endpoints,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> String {
    let scope = format!("{SCOPE_MAIL} {SCOPE_OFFLINE}");
    let q = |v: &str| urlencode(v);
    format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256&resource={}",
        endpoints.authorization_endpoint,
        q(client_id),
        q(redirect_uri),
        q(&scope),
        q(state),
        q(challenge),
        q(FASTMAIL_RESOURCE),
    )
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

pub async fn exchange_code(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    client_id: &str,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<Tokens> {
    post_token(
        http,
        endpoints,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("code_verifier", verifier),
            ("resource", FASTMAIL_RESOURCE),
        ],
    )
    .await
    .context("exchanging the authorization code")
}

pub async fn refresh(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    client_id: &str,
    refresh_token: &str,
) -> Result<Tokens> {
    post_token(
        http,
        endpoints,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("resource", FASTMAIL_RESOURCE),
        ],
    )
    .await
    .context("refreshing the access token")
}

async fn post_token(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    form: &[(&str, &str)],
) -> Result<Tokens> {
    let response = http
        .post(&endpoints.token_endpoint)
        .form(form)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        // The error body carries OAuth's own reason (invalid_grant and friends),
        // which is far more useful than the status alone.
        let detail = response.text().await.unwrap_or_default();
        anyhow::bail!("token endpoint returned {status}: {detail}");
    }
    Ok(response.json().await.context("parsing token response")?)
}

/// Waits on the loopback redirect for a single authorization code.
///
/// Binds before the browser is opened so the redirect can never arrive at a
/// closed port. Returns the code only if `state` matches, which is what stops
/// another page on the machine from feeding us a code of its choosing.
pub async fn wait_for_code(
    listener: tokio::net::TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let accept = async {
        loop {
            let (mut socket, _) = listener.accept().await?;

            let mut buf = vec![0u8; 8192];
            let read = socket.read(&mut buf).await?;
            let request = String::from_utf8_lossy(&buf[..read]).to_string();

            // "GET /callback?code=…&state=… HTTP/1.1"
            let Some(target) = request.split_whitespace().nth(1) else {
                continue;
            };
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");

            let mut code = None;
            let mut state = None;
            let mut error = None;
            let mut description = None;
            for pair in query.split('&') {
                match pair.split_once('=') {
                    Some(("code", v)) => code = Some(urldecode(v)),
                    Some(("state", v)) => state = Some(urldecode(v)),
                    Some(("error", v)) => error = Some(urldecode(v)),
                    Some(("error_description", v)) => description = Some(urldecode(v)),
                    _ => {}
                }
            }

            let outcome = if let Some(e) = error {
                match description {
                    Some(d) => Err(anyhow!("sign-in was refused: {e} — {d}")),
                    None => Err(anyhow!("sign-in was refused: {e}")),
                }
            } else if state.as_deref() != Some(expected_state) {
                Err(anyhow!("redirect state did not match — ignoring"))
            } else if let Some(code) = code {
                Ok(code)
            } else {
                Err(anyhow!("redirect carried no authorization code"))
            };

            let page = match &outcome {
                Ok(_) => CALLBACK_OK,
                Err(_) => CALLBACK_FAILED,
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                page.len(),
                page
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;

            return outcome;
        }
    };

    tokio::time::timeout(timeout, accept)
        .await
        .map_err(|_| anyhow!("timed out waiting for the browser redirect"))?
}

const CALLBACK_OK: &str = "<!doctype html><meta charset=utf-8><title>BazMail</title>\
<body style=\"background:#111827;color:#f8f9fb;font:16px/1.6 system-ui;display:grid;place-items:center;height:100vh;margin:0\">\
<div style=\"text-align:center\"><p style=\"font-size:20px;font-weight:600\">Connected.</p>\
<p style=\"color:#969ca6\">You can close this tab and go back to BazMail.</p></div>";

const CALLBACK_FAILED: &str = "<!doctype html><meta charset=utf-8><title>BazMail</title>\
<body style=\"background:#111827;color:#f8f9fb;font:16px/1.6 system-ui;display:grid;place-items:center;height:100vh;margin:0\">\
<div style=\"text-align:center\"><p style=\"font-size:20px;font-weight:600\">Sign-in failed.</p>\
<p style=\"color:#969ca6\">Close this tab and try again from BazMail.</p></div>";

fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn urldecode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&value[i + 1..i + 3], 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_the_sha256_of_the_verifier() {
        let p = pkce();
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(p.verifier.as_bytes()));
        assert_eq!(p.challenge, expected);
        // RFC 7636 requires 43-128 characters.
        assert!(p.verifier.len() >= 43 && p.verifier.len() <= 128);
    }

    #[test]
    fn each_pkce_pair_is_unique() {
        assert_ne!(pkce().verifier, pkce().verifier);
    }

    #[test]
    fn authorize_url_escapes_its_parameters() {
        let endpoints = Endpoints {
            registration_endpoint: "https://x/reg".into(),
            authorization_endpoint: "https://x/auth".into(),
            token_endpoint: "https://x/token".into(),
            revocation_endpoint: None,
        };
        let url = authorize_url(&endpoints, "abc", "http://localhost:41789/callback", "st", "ch");
        // A raw colon or slash in redirect_uri would truncate the query.
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A41789%2Fcallback"));
        assert!(url.contains("code_challenge_method=S256"));
        // Fastmail rejects the authorization outright without this.
        assert!(url.contains("resource=https%3A%2F%2Fapi.fastmail.com%2Fjmap%2Fsession"));
        // Scope is space-separated and must survive as %20.
        assert!(url.contains("scope=urn%3Aietf%3Aparams%3Aoauth%3Ascope%3Amail%20offline_access"));
    }

    #[test]
    fn urldecode_round_trips_what_urlencode_produces() {
        let raw = "a code/with+odd chars?&=";
        assert_eq!(urldecode(&urlencode(raw)), raw);
    }
}
