use anyhow::{anyhow, Result};
use axum::{extract::Query, routing::get, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};
// No server-side persistent storage
use serde::Deserialize;
use serde_json::Value;
use std::net::SocketAddr;
use std::process::Command;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::timeout;
use url::Url;
// No disk/config fallback
use is_terminal::IsTerminal;
use std::io::{self, Write};
use url::form_urlencoded;

const DEFAULT_SCOPES: &str = "read:user user:email";

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: String,
    state: String,
}

// Generate a random URL-safe string suitable for PKCE values
fn random_url_safe(len: usize) -> String {
    use rand::RngCore;
    let mut bytes = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub async fn ensure_authenticated() -> Result<()> {
    // Allow bypass in strictly controlled environments if needed
    if std::env::var("GOOSE_AUTH_BYPASS").unwrap_or_default() == "1" {
        return Ok(());
    }

    // Always prompt to log in (no persistent storage)
    println!("Please log in");
    if io::stdin().is_terminal() {
        let _ = io::stdout().flush();
        let mut _buf = String::new();
        let _ = io::stdin().read_line(&mut _buf);

        // Ask for mode
        print!("Select authentication mode: [a]utomatic (callback) / [m]anual (paste URL) [a]: ");
        let _ = io::stdout().flush();
        let mut choice = String::new();
        let _ = io::stdin().read_line(&mut choice);
        let choice = choice.trim().to_lowercase();
        if choice.starts_with('m') {
            return login_manual_only().await;
        }
    }
    // Default to automatic
    login().await
}

pub async fn login() -> Result<()> {
    let client_id = std::env::var("GOOSE_GITHUB_CLIENT_ID")
        .map_err(|_| anyhow!("GOOSE_GITHUB_CLIENT_ID is required for GitHub OAuth"))?;
    let redirect_url = std::env::var("GOOSE_AUTH_REDIRECT_URL")
        .map_err(|_| anyhow!("GOOSE_AUTH_REDIRECT_URL must be set to a stable HTTPS callback URL"))?;

    let scopes = std::env::var("GOOSE_GITHUB_SCOPES").unwrap_or_else(|_| DEFAULT_SCOPES.to_string());
    let client_secret = std::env::var("GOOSE_GITHUB_CLIENT_SECRET").ok();

    // PKCE S256 (required by GitHub)
    let state = random_url_safe(24);
    // Verifier must be 43-128 chars; 64 random bytes -> ~86 chars base64url
    let code_verifier = random_url_safe(64);
    let digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(digest);

    let mut auth_url = Url::parse("https://github.com/login/oauth/authorize")?;
    {
        let mut qp = auth_url.query_pairs_mut();
        qp.append_pair("response_type", "code");
        qp.append_pair("client_id", &client_id);
        qp.append_pair("redirect_uri", &redirect_url);
        qp.append_pair("scope", &scopes);
        qp.append_pair("state", &state);
        qp.append_pair("code_challenge", &code_challenge);
        qp.append_pair("code_challenge_method", "S256");
    }

    
    let listen_addr = std::env::var("GOOSE_AUTH_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listen_addr: SocketAddr = listen_addr.parse()?;

    // Channel to receive code
    let (tx, rx) = oneshot::channel::<(String, String)>();
    let expected_state = std::sync::Arc::new(state.clone());
    let expected_state_for_route = expected_state.clone();

    // Build a tiny router for /oauth_callback
    let app = {
        let tx_arc = std::sync::Arc::new(tokio::sync::Mutex::new(Some(tx)));
        Router::new().route(
            "/oauth_callback",
            get(move |Query(q): Query<CallbackQuery>| {
                let tx = tx_arc.clone();
                let expected_state = expected_state_for_route.clone();
                async move {
                    let body = if q.state == expected_state.as_ref().as_str() {
                        if let Some(sender) = tx.lock().await.take() {
                            let _ = sender.send((q.code.clone(), q.state.clone()));
                        }
                        "<html><body><h3>Authentication succeeded. You can close this window.</h3></body></html>"
                    } else {
                        "<html><body><h3>Invalid state parameter.</h3></body></html>"
                    };
                    axum::response::Html(body)
                }
            }),
        )
    };

    // Start server with shutdown when we get the code or timeout
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;

    println!("\nOpen this URL in your browser to continue:\n  {}\n", auth_url);
    
    let no_browser = std::env::var("GOOSE_NO_BROWSER").unwrap_or_default() == "1";
    if !no_browser {
        if let Err(e) = webbrowser::open(auth_url.as_str()) {
            eprintln!("[oauth-info] Could not open browser automatically: {}", e);
        }
    }

    // Start server as a background task and wait for callback (up to 60s)
    let server_task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let result = timeout(Duration::from_secs(60), rx).await;

    // Stop server
    server_task.abort();

    let (code, returned_state) = match result {
        Ok(Ok(pair)) => pair,
        Ok(Err(_)) => {
            eprintln!("[oauth-info] Did not capture OAuth callback automatically.");
            manual_oauth_input(expected_state.as_ref()).await?
        }
        Err(_) => {
            eprintln!("[oauth-info] OAuth callback timed out after 60s.");
            manual_oauth_input(expected_state.as_ref()).await?
        }
    };
    if returned_state != state {
        return Err(anyhow!("State mismatch in OAuth callback"));
    }

    // Exchange code for token using curl to avoid adding new HTTP client deps
    let mut form: Vec<(&str, &str)> = Vec::new();
    form.push(("client_id", &client_id));
    form.push(("redirect_uri", &redirect_url));
    form.push(("grant_type", "authorization_code"));
    form.push(("code", &code));
    form.push(("code_verifier", &code_verifier));
    if let Some(ref secret) = client_secret {
        form.push(("client_secret", secret));
    }

    let mut args: Vec<String> = vec![
        "-s".into(),
        "-X".into(),
        "POST".into(),
        "-H".into(),
        "Accept: application/json".into(),
        "-H".into(),
        "Content-Type: application/x-www-form-urlencoded".into(),
        "https://github.com/login/oauth/access_token".into(),
    ];
    for (k, v) in form.iter() {
        args.push("--data-urlencode".into());
        args.push(format!("{}={}", k, v));
    }

    let output = Command::new("curl").args(&args).output();
    let output = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            return Err(anyhow!("Token exchange failed: {}", stderr));
        }
        Err(e) => return Err(anyhow!("Failed to run curl: {}", e)),
    };
            
    let json: Value = match serde_json::from_str(&output) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[oauth-debug] Raw token response (non-JSON): {}", output);
            return Err(anyhow!("Failed to parse token response as JSON: {}", e));
        }
    };

    // Log redacted response for debugging when access_token is missing
    let access_token = match json.get("access_token").and_then(|v| v.as_str()) {
        Some(tok) => tok,
        None => {
            // Redact sensitive fields if present, print the rest
            let mut redacted = json.clone();
            if let Some(obj) = redacted.as_object_mut() {
                if obj.contains_key("access_token") {
                    obj.insert("access_token".to_string(), Value::String("<redacted>".to_string()));
                }
                if obj.contains_key("refresh_token") {
                    obj.insert("refresh_token".to_string(), Value::String("<redacted>".to_string()));
                }
            }
            eprintln!("[oauth-debug] Token endpoint response (redacted): {}", serde_json::to_string_pretty(&redacted).unwrap_or_else(|_| "<unprintable>".into()));
            eprintln!("[oauth-debug] Used redirect_uri: {}", redirect_url);
            eprintln!("[oauth-debug] Used scopes: {}", scopes);
            eprintln!("[oauth-debug] Client ID present: {}", !client_id.is_empty());
            eprintln!("[oauth-debug] Client secret provided: {}", client_secret.is_some());
            return Err(anyhow!("No access_token in token response"));
        }
    };

    // Do not persist token; just validate successful retrieval
    println!("Login successful (token validated, not persisted)");
    Ok(())
}

// Explicit interactive login helper for `goose auth login` without flags
pub async fn login_interactive() -> Result<()> {
    if io::stdin().is_terminal() {
        println!("Select authentication mode:");
        println!("  1) Automatic (callback server)");
        println!("  2) Manual (paste redirected URL)");
        print!("Enter choice [1]: ");
        let _ = io::stdout().flush();
        let mut choice = String::new();
        let _ = io::stdin().read_line(&mut choice);
        let c = choice.trim();
        if c == "2" || c.eq_ignore_ascii_case("m") {
            return login_manual_only().await;
        }
    }
    // Default automatic
    login().await
}

pub async fn login_manual_only() -> Result<()> {
    let client_id = std::env::var("GOOSE_GITHUB_CLIENT_ID")
        .map_err(|_| anyhow!("GOOSE_GITHUB_CLIENT_ID is required for GitHub OAuth"))?;
    let redirect_url = std::env::var("GOOSE_AUTH_REDIRECT_URL")
        .map_err(|_| anyhow!("GOOSE_AUTH_REDIRECT_URL must be set to a stable HTTPS callback URL"))?;

    let scopes = std::env::var("GOOSE_GITHUB_SCOPES").unwrap_or_else(|_| DEFAULT_SCOPES.to_string());
    let client_secret = std::env::var("GOOSE_GITHUB_CLIENT_SECRET").ok();

    // PKCE S256
    let state = random_url_safe(24);
    let code_verifier = random_url_safe(64);
    let digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(digest);

    let mut auth_url = Url::parse("https://github.com/login/oauth/authorize")?;
    {
        let mut qp = auth_url.query_pairs_mut();
        qp.append_pair("response_type", "code");
        qp.append_pair("client_id", &client_id);
        qp.append_pair("redirect_uri", &redirect_url);
        qp.append_pair("scope", &scopes);
        qp.append_pair("state", &state);
        qp.append_pair("code_challenge", &code_challenge);
        qp.append_pair("code_challenge_method", "S256");
    }

    println!("\nManual authentication selected. Open this URL:\n  {}\n", auth_url);
    let no_browser = std::env::var("GOOSE_NO_BROWSER").unwrap_or_default() == "1";
    if !no_browser {
        let _ = webbrowser::open(auth_url.as_str());
    }
    let (code, returned_state) = manual_oauth_input(&state).await?;
    if returned_state != state {
        return Err(anyhow!("State mismatch in OAuth callback (manual)"));
    }

    // Exchange token (duplicate of above logic for clarity)
    let mut form: Vec<(&str, &str)> = Vec::new();
    form.push(("client_id", &client_id));
    form.push(("redirect_uri", &redirect_url));
    form.push(("grant_type", "authorization_code"));
    form.push(("code", &code));
    form.push(("code_verifier", &code_verifier));
    if let Some(ref secret) = client_secret {
        form.push(("client_secret", secret));
    }

    let mut args: Vec<String> = vec![
        "-s".into(),
        "-X".into(),
        "POST".into(),
        "-H".into(),
        "Accept: application/json".into(),
        "-H".into(),
        "Content-Type: application/x-www-form-urlencoded".into(),
        "https://github.com/login/oauth/access_token".into(),
    ];
    for (k, v) in form.iter() {
        args.push("--data-urlencode".into());
        args.push(format!("{}={}", k, v));
    }

    let output = Command::new("curl").args(&args).output();
    let output = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            return Err(anyhow!("Token exchange failed: {}", stderr));
        }
        Err(e) => return Err(anyhow!("Failed to run curl: {}", e)),
    };

    let json: Value = match serde_json::from_str(&output) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[oauth-debug] Raw token response (non-JSON): {}", output);
            return Err(anyhow!("Failed to parse token response as JSON: {}", e));
        }
    };

    let access_token = match json.get("access_token").and_then(|v| v.as_str()) {
        Some(tok) => tok,
        None => {
            let mut redacted = json.clone();
            if let Some(obj) = redacted.as_object_mut() {
                if obj.contains_key("access_token") {
                    obj.insert("access_token".to_string(), Value::String("<redacted>".to_string()));
                }
                if obj.contains_key("refresh_token") {
                    obj.insert("refresh_token".to_string(), Value::String("<redacted>".to_string()));
                }
            }
            eprintln!("[oauth-debug] Token endpoint response (redacted): {}", serde_json::to_string_pretty(&redacted).unwrap_or_else(|_| "<unprintable>".into()));
            eprintln!("[oauth-debug] Used redirect_uri: {}", redirect_url);
            eprintln!("[oauth-debug] Used scopes: {}", scopes);
            eprintln!("[oauth-debug] Client ID present: {}", !client_id.is_empty());
            eprintln!("[oauth-debug] Client secret provided: {}", client_secret.is_some());
            return Err(anyhow!("No access_token in token response"));
        }
    };

    // End of manual flow legacy path
    println!("Login successful");
    Ok(())
}

async fn manual_oauth_input(expected_state: &str) -> Result<(String, String)> {
    if !io::stdin().is_terminal() {
        return Err(anyhow!(
            "No interactive input available. Re-run with a TTY or set GOOSE_NO_BROWSER=1 and paste the code when prompted."
        ));
    }

    println!("\nManual OAuth fallback");
    println!("1) Open the printed URL in your browser");
    println!("2) After authorizing, copy either:");
    println!("   - the full redirected URL you land on, OR");
    println!("   - just the value of the 'code' parameter");
    print!("Paste here and press Enter: ");
    let _ = io::stdout().flush();

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if let Ok(url) = Url::parse(input) {
        let mut code: Option<String> = None;
        let mut state: Option<String> = None;
        for (k, v) in url.query_pairs() {
            if k == "code" {
                code = Some(v.to_string());
            } else if k == "state" {
                state = Some(v.to_string());
            }
        }
        if let Some(code) = code {
            let returned_state = state.unwrap_or_else(|| expected_state.to_string());
            if returned_state != expected_state {
                return Err(anyhow!("State mismatch in pasted URL"));
            }
            return Ok((code, returned_state));
        }
    }

    if input.contains('=') && input.contains('&') {
        let mut code: Option<String> = None;
        let mut state: Option<String> = None;
        for (k, v) in form_urlencoded::parse(input.as_bytes()) {
            if k == "code" {
                code = Some(v.into_owned());
            } else if k == "state" {
                state = Some(v.into_owned());
            }
        }
        if let Some(code) = code {
            let returned_state = state.unwrap_or_else(|| expected_state.to_string());
            if returned_state != expected_state {
                return Err(anyhow!("State mismatch in pasted parameters"));
            }
            return Ok((code, returned_state));
        }
    }

    if !input.is_empty() {
        return Ok((input.to_string(), expected_state.to_string()));
    }

    Err(anyhow!("No code provided"))
}

pub async fn status() -> Result<()> {
    // Force re-login: do not consider any in-memory token
    println!("Not authenticated. Run: goose auth login");
    Ok(())
}

pub async fn logout() -> Result<()> {
    // No server-side persistent storage; advise user to clear browser cookies
    println!("Logged out. If you used the browser, clear site cookies to remove that session.");
    Ok(())
}
