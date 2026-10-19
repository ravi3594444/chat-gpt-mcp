use axum::{
    extract::{Form, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn hash(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}
fn equal(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Clone)]
pub struct Auth {
    pub public_url: String,
    admin_key: String,
    path: PathBuf,
    store: Arc<Mutex<Store>>,
}
#[derive(Default, Serialize, Deserialize)]
struct Store {
    clients: HashMap<String, Client>,
    access: HashMap<String, Token>,
    refresh: HashMap<String, Token>,
    devices: HashMap<String, String>,
    #[serde(skip)]
    approvals: HashMap<String, Approval>,
    #[serde(skip)]
    codes: HashMap<String, Code>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Client {
    redirect_uris: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Token {
    client_id: String,
    expires: u64,
}
struct Approval {
    request: Authorize,
    expires: u64,
}
struct Code {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    expires: u64,
}

impl Auth {
    pub fn load(public_url: String, dir: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        let key_path = dir.join("admin.key");
        let admin_key = if key_path.exists() {
            fs::read_to_string(&key_path)?.trim().to_string()
        } else {
            let key = random_token();
            private_write(&key_path, key.as_bytes())?;
            key
        };
        if admin_key.len() < 32 {
            return Err("admin.key must contain at least 32 characters".into());
        }
        let path = dir.join("auth.json");
        let store = if path.exists() {
            serde_json::from_slice(&fs::read(&path)?)?
        } else {
            Store::default()
        };
        Ok(Self {
            public_url,
            admin_key,
            path,
            store: Arc::new(Mutex::new(store)),
        })
    }
    fn save(&self, store: &Store) -> Result<(), StatusCode> {
        private_write(
            &self.path,
            &serde_json::to_vec(store).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }
    pub fn admin_key(&self) -> &str {
        &self.admin_key
    }
    pub fn valid_access(&self, token: &str) -> bool {
        self.store
            .lock()
            .unwrap()
            .access
            .get(&hash(token))
            .is_some_and(|t| t.expires > now())
    }
    pub fn device_name(&self, token: &str) -> Option<String> {
        self.store
            .lock()
            .unwrap()
            .devices
            .get(&hash(token))
            .cloned()
    }
}

fn private_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let temp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temp, path)
}

pub async fn metadata(State(auth): State<Auth>) -> Json<Value> {
    let base = auth.public_url;
    Json(
        json!({"issuer":base,"authorization_endpoint":format!("{base}/oauth/authorize"),"token_endpoint":format!("{base}/oauth/token"),"registration_endpoint":format!("{base}/oauth/register"),"revocation_endpoint":format!("{base}/oauth/revoke"),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"token_endpoint_auth_methods_supported":["none"],"code_challenge_methods_supported":["S256"],"scopes_supported":["browser:control"]}),
    )
}
pub async fn resource(State(auth): State<Auth>) -> Json<Value> {
    Json(
        json!({"resource":format!("{}/mcp",auth.public_url),"authorization_servers":[auth.public_url],"scopes_supported":["browser:control"],"bearer_methods_supported":["header"]}),
    )
}

#[derive(Deserialize)]
pub struct Register {
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: Option<String>,
}
pub async fn register(
    State(auth): State<Auth>,
    Json(input): Json<Register>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    if input.redirect_uris.is_empty()
        || input.redirect_uris.len() > 8
        || input
            .token_endpoint_auth_method
            .as_deref()
            .is_some_and(|m| m != "none")
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    for uri in &input.redirect_uris {
        let url = url::Url::parse(uri).map_err(|_| StatusCode::BAD_REQUEST)?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        let chatgpt = matches!(url.host_str(), Some("chatgpt.com" | "chat.openai.com"));
        if !(local && url.scheme() == "http" || chatgpt && url.scheme() == "https")
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let client_id = random_token();
    let mut store = auth.store.lock().unwrap();
    if store.clients.len() >= 256 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    store.clients.insert(
        client_id.clone(),
        Client {
            redirect_uris: input.redirect_uris.clone(),
        },
    );
    auth.save(&store)?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"client_id":client_id,"redirect_uris":input.redirect_uris,"token_endpoint_auth_method":"none","grant_types":["authorization_code","refresh_token"],"response_types":["code"]}),
        ),
    ))
}

#[derive(Deserialize)]
pub struct Authorize {
    client_id: String,
    redirect_uri: String,
    response_type: String,
    code_challenge: String,
    code_challenge_method: String,
    state: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
}
pub async fn authorize(
    State(auth): State<Auth>,
    Query(input): Query<Authorize>,
) -> Result<Response, StatusCode> {
    let mut store = auth.store.lock().unwrap();
    let client = store
        .clients
        .get(&input.client_id)
        .ok_or(StatusCode::BAD_REQUEST)?;
    if !client.redirect_uris.contains(&input.redirect_uri)
        || input.response_type != "code"
        || input.code_challenge_method != "S256"
        || input.code_challenge.len() != 43
        || input
            .scope
            .as_deref()
            .is_some_and(|s| s != "browser:control")
        || input
            .resource
            .as_deref()
            .is_some_and(|r| r != format!("{}/mcp", auth.public_url))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    store.approvals.retain(|_, r| r.expires > now());
    if store.approvals.len() >= 128 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let nonce = random_token();
    let redirect = escape(&input.redirect_uri);
    // Chrome applies form-action to the redirect following approval as well.
    // Permit only the callback origin registered for this specific request.
    let callback_origin = url::Url::parse(&input.redirect_uri)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .origin()
        .ascii_serialization();
    let policy = format!("default-src 'none'; style-src 'self'; form-action 'self' {callback_origin}; frame-ancestors 'none'; base-uri 'none'");
    store.approvals.insert(
        nonce.clone(),
        Approval {
            request: input,
            expires: now() + 300,
        },
    );
    Ok(([(header::CONTENT_SECURITY_POLICY, policy)], Html(format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Connect Browser Link</title><link rel="stylesheet" href="/style.css"></head><body><main><span class="tag">BROWSER LINK · PERSONAL</span><h1>Connect your browser.</h1><p>Allow this ChatGPT connection to use tabs you enable in the Browser Link extension. You can stop access in the extension at any time.</p><form method="post" action="/oauth/approve"><input type="hidden" name="nonce" value="{nonce}"><label for="key">Your Browser Link setup key</label><input id="key" type="password" name="key" autocomplete="off" required autofocus><button>Connect to ChatGPT</button></form><p class="muted">Find the key in your server’s data/admin.key file. Enter it here, never in chat.</p><p class="muted">Return address: {redirect}</p></main></body></html>"#
    ))).into_response())
}
#[derive(Deserialize)]
pub struct Approve {
    nonce: String,
    key: String,
}

fn approval_error(status: StatusCode, code: &str, message: &str) -> Response {
    // Log a reason only, never the setup key, nonce, callback, or OAuth tokens.
    eprintln!("OAuth approval rejected: {code}");
    (status, Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Browser Link connection</title><link rel=\"stylesheet\" href=\"/style.css\"></head><body><main><span class=\"tag\">BROWSER LINK</span><h1>Connection needs another attempt.</h1><p>{}</p><p>Return to ChatGPT and start a new Browser Link connection. Submit the new approval form once.</p><p class=\"muted\">If a fresh attempt fails, share this error code: <code>{}</code></p></main></body></html>",
        escape(message), escape(code)
    ))).into_response()
}

pub async fn approve(State(auth): State<Auth>, Form(input): Form<Approve>) -> Response {
    if !equal(&input.key, &auth.admin_key) {
        return approval_error(
            StatusCode::UNAUTHORIZED,
            "incorrect_setup_key",
            "The setup key did not match this bridge.",
        );
    }
    let mut store = auth.store.lock().unwrap();
    let approval = match store.approvals.remove(&input.nonce) {
        Some(approval) if approval.expires > now() => approval,
        Some(_) => {
            return approval_error(
                StatusCode::BAD_REQUEST,
                "approval_request_expired",
                "This approval request expired.",
            )
        }
        None => {
            return approval_error(
                StatusCode::BAD_REQUEST,
                "approval_request_missing",
                "This approval request is unavailable or has already been submitted.",
            )
        }
    };
    let input = approval.request;
    let code = random_token();
    store.codes.insert(
        hash(&code),
        Code {
            client_id: input.client_id,
            redirect_uri: input.redirect_uri.clone(),
            challenge: input.code_challenge,
            expires: now() + 120,
        },
    );
    let mut redirect = match url::Url::parse(&input.redirect_uri) {
        Ok(redirect) => redirect,
        Err(_) => {
            return approval_error(
                StatusCode::BAD_REQUEST,
                "invalid_callback",
                "The registered return address was invalid.",
            )
        }
    };
    redirect.query_pairs_mut().append_pair("code", &code);
    if let Some(state) = input.state {
        redirect.query_pairs_mut().append_pair("state", &state);
    }
    Redirect::to(redirect.as_str()).into_response()
}

#[derive(Deserialize)]
pub struct Exchange {
    grant_type: String,
    client_id: String,
    code: Option<String>,
    code_verifier: Option<String>,
    redirect_uri: Option<String>,
    refresh_token: Option<String>,
    resource: Option<String>,
}
pub async fn token(
    State(auth): State<Auth>,
    Form(input): Form<Exchange>,
) -> Result<impl IntoResponse, StatusCode> {
    if input
        .resource
        .as_deref()
        .is_some_and(|r| r != format!("{}/mcp", auth.public_url))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut store = auth.store.lock().unwrap();
    match input.grant_type.as_str() {
        "authorization_code" => {
            let code = store
                .codes
                .remove(&hash(input.code.as_deref().unwrap_or_default()))
                .ok_or(StatusCode::BAD_REQUEST)?;
            let verifier = input.code_verifier.as_deref().unwrap_or_default();
            if code.expires <= now()
                || code.client_id != input.client_id
                || input.redirect_uri.as_deref() != Some(&code.redirect_uri)
                || !(43..=128).contains(&verifier.len())
                || !equal(&hash(verifier), &code.challenge)
            {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
        "refresh_token" => {
            let key = hash(input.refresh_token.as_deref().unwrap_or_default());
            let token = store.refresh.get(&key).ok_or(StatusCode::BAD_REQUEST)?;
            if token.expires <= now() || token.client_id != input.client_id {
                return Err(StatusCode::BAD_REQUEST);
            }
            store.refresh.remove(&key);
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    }
    store.access.retain(|_, t| t.expires > now());
    store.refresh.retain(|_, t| t.expires > now());
    let access = random_token();
    let refresh = random_token();
    store.access.insert(
        hash(&access),
        Token {
            client_id: input.client_id.clone(),
            expires: now() + 3600,
        },
    );
    store.refresh.insert(
        hash(&refresh),
        Token {
            client_id: input.client_id,
            expires: now() + 30 * 86400,
        },
    );
    auth.save(&store)?;
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(
            json!({"access_token":access,"token_type":"Bearer","expires_in":3600,"refresh_token":refresh,"scope":"browser:control"}),
        ),
    ))
}

#[derive(Deserialize)]
pub struct Revoke {
    token: String,
    client_id: String,
}
pub async fn revoke(
    State(auth): State<Auth>,
    Form(input): Form<Revoke>,
) -> Result<StatusCode, StatusCode> {
    let mut store = auth.store.lock().unwrap();
    let key = hash(&input.token);
    if store
        .access
        .get(&key)
        .is_some_and(|t| t.client_id == input.client_id)
    {
        store.access.remove(&key);
    }
    if store
        .refresh
        .get(&key)
        .is_some_and(|t| t.client_id == input.client_id)
    {
        store.refresh.remove(&key);
    }
    auth.save(&store)?;
    Ok(StatusCode::OK)
}
#[derive(Deserialize)]
pub struct Pair {
    name: String,
}
pub async fn pair(
    State(auth): State<Auth>,
    headers: HeaderMap,
    Json(input): Json<Pair>,
) -> Result<Json<Value>, StatusCode> {
    let key = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !equal(key, &auth.admin_key) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let token = random_token();
    let mut store = auth.store.lock().unwrap();
    // A personal bridge has one paired device. Re-pairing invalidates previous device credentials.
    store.devices.clear();
    store
        .devices
        .insert(hash(&token), input.name.chars().take(80).collect());
    auth.save(&store)?;
    Ok(Json(json!({"device_token":token})))
}

pub async fn unpair(
    State(auth): State<Auth>,
    headers: HeaderMap,
) -> Result<StatusCode, StatusCode> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let mut store = auth.store.lock().unwrap();
    if store.devices.remove(&hash(token)).is_none() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    auth.save(&store)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unauthorized(auth: &Auth) -> Response {
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error":"Authentication required"})),
    )
        .into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        format!(
            r#"Bearer resource_metadata="{}/.well-known/oauth-protected-resource/mcp""#,
            auth.public_url
        )
        .parse()
        .unwrap(),
    );
    response
}
