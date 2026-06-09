use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use tower_http::cors::CorsLayer;

// ---------------------------------------------------------------------------
// The gateway is a thin edge proxy. It forwards OCR submit/poll requests to the
// job-service, which owns all Kafka and Postgres interaction. Keeping this layer
// stateless means it can be scaled freely and holds no job state of its own.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    job_service_url: String,
}

async fn health() -> &'static str {
    "ok"
}

/// POST /ocr → POST {job_service}/jobs
async fn submit_ocr(State(state): State<AppState>, body: Bytes) -> Response {
    let url = format!("{}/jobs", state.job_service_url);
    forward(state.client.post(&url).body(body), "submit").await
}

/// GET /ocr/:job_id → GET {job_service}/jobs/:job_id
async fn poll_ocr(Path(job_id): Path<String>, State(state): State<AppState>) -> Response {
    let url = format!("{}/jobs/{}", state.job_service_url, job_id);
    forward(state.client.get(&url), "poll").await
}

/// Execute an upstream request and mirror its status + body back to the caller.
async fn forward(req: reqwest::RequestBuilder, op: &str) -> Response {
    let req = req.header("content-type", "application/json");
    match req.send().await {
        Ok(resp) => {
            let status =
                StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = resp.bytes().await.unwrap_or_default();
            (status, body).into_response()
        }
        Err(e) => {
            tracing::error!("{op}: upstream request failed: {e}");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("api_gateway=debug".parse().unwrap()),
        )
        .init();

    let job_service_url = std::env::var("JOB_SERVICE_URL")
        .unwrap_or_else(|_| "http://localhost:8081".into());

    let state = AppState {
        client: reqwest::Client::new(),
        job_service_url,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/ocr", post(submit_ocr))
        .route("/ocr/:job_id", get(poll_ocr))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = "0.0.0.0:8080";
    tracing::info!("listening on {addr}, forwarding to {}", "job-service");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
