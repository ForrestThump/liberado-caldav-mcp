mod caldav;
mod models;
mod server;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let state = std::sync::Arc::new(models::AppState {
        caldav_base_url: std::env::var("CALDAV_URL")
            .unwrap_or_else(|_| "http://baikal/dav.php".to_string()),
        username: std::env::var("CALDAV_USERNAME")
            .unwrap_or_else(|_| "admin".to_string()),
        password: std::env::var("CALDAV_PASSWORD").unwrap_or_default(),
        api_key: std::env::var("CALDAV_MCP_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty()),
        http_client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client"),
    });

    let app = server::create_app(state);
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8011".to_string());
    tracing::info!("caldav-mcp listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind");
    axum::serve(listener, app).await.unwrap();
}
