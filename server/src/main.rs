mod bridge;
mod mcp;
mod oauth;

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Request, State, WebSocketUpgrade},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bridge::Bridge;
use oauth::Auth;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde_json::json;
use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind: SocketAddr = env::var("BROWSER_LINK_BIND")
        .unwrap_or("127.0.0.1:7319".into())
        .parse()?;
    let public_url = env::var("BROWSER_LINK_PUBLIC_URL")
        .unwrap_or_else(|_| format!("http://127.0.0.1:{}", bind.port()))
        .trim_end_matches('/')
        .to_string();
    let url = url::Url::parse(&public_url)?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if (!local && url.scheme() != "https")
        || !matches!(url.scheme(), "http" | "https")
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "BROWSER_LINK_PUBLIC_URL must be a HTTPS origin (HTTP is allowed on localhost)".into(),
        );
    }
    let dir = PathBuf::from(env::var("BROWSER_LINK_DATA_DIR").unwrap_or("data".into()));
    let auth = Auth::load(public_url.clone(), dir.clone())?;
    if env::args().any(|a| a == "--show-key") {
        println!("{}", auth.admin_key());
        return Ok(());
    }
    let bridge = Bridge::new();
    let mcp_bridge = bridge.clone();
    let mut mcp_config = StreamableHttpServerConfig::default().with_allowed_hosts([
        url.host_str().unwrap_or("127.0.0.1"),
        "127.0.0.1",
        "localhost",
    ]);
    mcp_config.legacy_session_mode = false;
    mcp_config.json_response = true;
    let service = StreamableHttpService::new(
        move || Ok(mcp::BrowserMcp::new(mcp_bridge.clone())),
        Arc::new(LocalSessionManager::default()),
        mcp_config,
    );
    let mcp_routes = Router::new()
        .nest_service("/mcp", service)
        .route_layer(middleware::from_fn_with_state(auth.clone(), require_auth));
    let ws_bridge = bridge.clone();
    let ws_auth = auth.clone();
    let app=Router::new()
        .route("/",get(||async{Html("<!doctype html><html><head><meta charset='utf-8'><title>Browser Link</title><link rel='stylesheet' href='/style.css'></head><body><main><span class='tag'>BROWSER LINK</span><h1>Your browser, connected.</h1><p>The Rust bridge is running. Pair the Browser Link extension using your setup key, then connect ChatGPT to <code>/mcp</code> using OAuth.</p><p>Only tabs you enable in the extension are accessible.</p></main></body></html>")}))
        .route("/style.css",get(||async{([(header::CONTENT_TYPE,"text/css")],include_str!("../assets/style.css"))}))
        .route("/health",get(||async{Json(json!({"status":"ok","version":env!("CARGO_PKG_VERSION")}))}))
        .route("/.well-known/oauth-authorization-server",get(oauth::metadata))
        .route("/.well-known/oauth-protected-resource",get(oauth::resource))
        .route("/.well-known/oauth-protected-resource/mcp",get(oauth::resource))
        .route("/oauth/register",post(oauth::register))
        .route("/oauth/authorize",get(oauth::authorize))
        .route("/oauth/approve",post(oauth::approve))
        .route("/oauth/token",post(oauth::token))
        .route("/oauth/revoke",post(oauth::revoke))
        .route("/pair",post(oauth::pair))
        .route("/unpair",post(oauth::unpair))
        .route("/bridge",get(move |ws:WebSocketUpgrade| { let bridge=ws_bridge.clone();let auth=ws_auth.clone();async move {ws.max_message_size(8*1024*1024).on_upgrade(move |socket|bridge.socket(socket,auth))} }))
        .with_state(auth.clone())
        .merge(mcp_routes)
        .layer(DefaultBodyLimit::max(64*1024))
        .layer(middleware::from_fn_with_state(auth,origin_and_headers));
    eprintln!(
        "Browser Link {} listening on {bind}",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("MCP endpoint: {public_url}/mcp (OAuth)");
    eprintln!(
        "Setup key: {} (keep it private). Run with --show-key to display it.",
        dir.join("admin.key").display()
    );
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn require_auth(State(auth): State<Auth>, request: Request<Body>, next: Next) -> Response {
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !auth.valid_access(token) {
        return oauth::unauthorized(&auth).await;
    }
    next.run(request).await
}

async fn origin_and_headers(
    State(auth): State<Auth>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let origin = origin.to_str().unwrap_or_default();
        let extension = url::Url::parse(origin).is_ok_and(|u| {
            u.scheme() == "chrome-extension"
                && u.host_str()
                    .is_some_and(|h| h.len() == 32 && h.bytes().all(|b| (b'a'..=b'p').contains(&b)))
                && (u.path().is_empty() || u.path() == "/")
        });
        if origin != auth.public_url
            && !extension
            && origin != "https://chatgpt.com"
            && origin != "https://chat.openai.com"
        {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    // HTML form POSTs use Origin: null under no-referrer. Preserve the origin
    // for our approval form while withholding referrers from other sites.
    headers.insert(header::REFERRER_POLICY, "same-origin".parse().unwrap());
    headers.entry(header::CONTENT_SECURITY_POLICY).or_insert_with(||
        "default-src 'none'; style-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'".parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}
