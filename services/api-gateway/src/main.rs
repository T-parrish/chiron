use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use dashmap::DashMap;
use rdkafka::{
    consumer::{Consumer, StreamConsumer},
    producer::{FutureProducer, FutureRecord},
    ClientConfig, Message,
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::oneshot;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

type PendingJobs = Arc<DashMap<String, oneshot::Sender<OcrResult>>>;

#[derive(Clone)]
struct AppState {
    producer: Arc<FutureProducer>,
    pending: PendingJobs,
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct OcrRequest {
    // base64-encoded image
    image_b64: String,
}

#[derive(Serialize)]
struct SubmitResponse {
    job_id: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct OcrResult {
    job_id: String,
    text: String,
    confidence: f32,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum PollResponse {
    Pending,
    Complete(OcrResult),
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> &'static str {
    "ok"
}

async fn submit_ocr(
    State(state): State<AppState>,
    Json(req): Json<OcrRequest>,
) -> Result<Json<SubmitResponse>, StatusCode> {
    let job_id = Uuid::new_v4().to_string();

    let payload = serde_json::json!({
        "job_id": job_id,
        "image_b64": req.image_b64,
    })
    .to_string();

    state
        .producer
        .send(
            FutureRecord::to("ocr.jobs")
                .key(&job_id)
                .payload(&payload),
            Duration::from_secs(5),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(SubmitResponse { job_id }))
}

async fn poll_ocr(
    Path(job_id): Path<String>,
    State(state): State<AppState>,
) -> Json<PollResponse> {
    // completed results land in a separate map populated by the consumer task
    if let Some((_, result)) = state.pending.remove(&format!("done:{job_id}")) {
        // oneshot::Sender is stored for pending; reuse DashMap with a prefix key
        // for completed results in a real impl use a second DashMap<String, OcrResult>
        drop(result);
    }
    Json(PollResponse::Pending)
}

// ---------------------------------------------------------------------------
// Kafka result consumer
// ---------------------------------------------------------------------------

async fn run_result_consumer(pending: PendingJobs, brokers: String) {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &brokers)
        .set("group.id", "api-gateway")
        .set("auto.offset.reset", "latest")
        .create()
        .expect("failed to create result consumer");

    consumer
        .subscribe(&["ocr.results"])
        .expect("failed to subscribe to ocr.results");

    loop {
        match consumer.recv().await {
            Ok(msg) => {
                if let Some(payload) = msg.payload() {
                    if let Ok(result) =
                        serde_json::from_slice::<OcrResult>(payload)
                    {
                        if let Some((_, tx)) =
                            pending.remove(&result.job_id)
                        {
                            let _ = tx.send(result);
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("kafka recv error: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("api_gateway=debug".parse().unwrap()),
        )
        .init();

    let brokers =
        std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "localhost:9092".into());

    let producer: FutureProducer = ClientConfig::new()
        .set("bootstrap.servers", &brokers)
        .create()
        .expect("failed to create kafka producer");

    let pending: PendingJobs = Arc::new(DashMap::new());

    tokio::spawn(run_result_consumer(Arc::clone(&pending), brokers));

    let state = AppState {
        producer: Arc::new(producer),
        pending,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/ocr", post(submit_ocr))
        .route("/ocr/{job_id}", get(poll_ocr))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = "0.0.0.0:8080";
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
