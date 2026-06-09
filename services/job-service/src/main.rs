use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use rdkafka::{
    consumer::{Consumer, StreamConsumer},
    producer::{FutureProducer, FutureRecord},
    ClientConfig, Message,
};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use std::time::Duration;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AppState {
    pool: PgPool,
    producer: FutureProducer,
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CreateJob {
    image_b64: String,
}

#[derive(Serialize)]
struct CreateJobResponse {
    job_id: String,
}

/// Shape consumed directly by the frontend.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum JobStatus {
    Pending,
    Complete {
        job_id: String,
        text: String,
        confidence: f32,
    },
    Failed {
        job_id: String,
        error: String,
    },
}

/// Result message published by the OCR worker on `ocr.results`.
#[derive(Deserialize)]
struct OcrResult {
    job_id: String,
    text: String,
    confidence: f32,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> &'static str {
    "ok"
}

/// Transactional submit:
///   BEGIN
///   INSERT job (status=pending)        -- uncommitted row holds the lock
///   publish to ocr.jobs
///   UPDATE job (status=in_progress)    -- only if publish succeeded
///   COMMIT                             -- otherwise the tx drops -> ROLLBACK
async fn create_job(
    State(state): State<AppState>,
    Json(req): Json<CreateJob>,
) -> Result<Json<CreateJobResponse>, StatusCode> {
    let job_id = Uuid::new_v4();

    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!("failed to begin tx: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Insert the pending row. Because the transaction is uncommitted, this row
    // is exclusively locked until we commit or roll back.
    sqlx::query("INSERT INTO jobs (id, status, image_b64) VALUES ($1, 'pending', $2)")
        .bind(job_id)
        .bind(&req.image_b64)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("insert failed for {job_id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Attempt to publish the job payload to Kafka.
    let payload = serde_json::json!({
        "job_id": job_id.to_string(),
        "image_b64": req.image_b64,
    })
    .to_string();
    let key = job_id.to_string();

    let publish = state
        .producer
        .send(
            FutureRecord::to("ocr.jobs").key(&key).payload(&payload),
            Duration::from_secs(5),
        )
        .await;

    if let Err((err, _)) = publish {
        // Publish failed → roll everything back and surface the error.
        tracing::error!("publish failed for {job_id}: {err}; rolling back");
        // Dropping tx without commit rolls back, but be explicit.
        let _ = tx.rollback().await;
        return Err(StatusCode::BAD_GATEWAY);
    }

    // Publish succeeded → mark in_progress and commit.
    sqlx::query("UPDATE jobs SET status = 'in_progress', updated_at = now() WHERE id = $1")
        .bind(job_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("status update failed for {job_id}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("commit failed for {job_id}: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::info!("created job {job_id}");
    Ok(Json(CreateJobResponse {
        job_id: job_id.to_string(),
    }))
}

async fn get_job(
    Path(job_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<JobStatus>, StatusCode> {
    let id = Uuid::parse_str(&job_id).map_err(|_| StatusCode::BAD_REQUEST)?;

    let row = sqlx::query(
        "SELECT status, result_text, confidence, error FROM jobs WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!("query failed for {job_id}: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

    let status: String = row.get("status");
    let job = match status.as_str() {
        "complete" => JobStatus::Complete {
            job_id,
            text: row.try_get("result_text").unwrap_or_default(),
            confidence: row.try_get("confidence").unwrap_or(0.0),
        },
        "failed" => JobStatus::Failed {
            job_id,
            error: row
                .try_get("error")
                .unwrap_or_else(|_| "unknown error".to_string()),
        },
        // pending or in_progress both read as "pending" to the frontend
        _ => JobStatus::Pending,
    };

    Ok(Json(job))
}

// ---------------------------------------------------------------------------
// Kafka result consumer → Postgres
// ---------------------------------------------------------------------------

async fn run_result_consumer(pool: PgPool, brokers: String) {
    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &brokers)
        .set("group.id", "job-service")
        .set("auto.offset.reset", "earliest")
        .set("fetch.message.max.bytes", "10485760")
        .create()
        .expect("failed to create result consumer");

    consumer
        .subscribe(&["ocr.results"])
        .expect("failed to subscribe to ocr.results");

    tracing::info!("result consumer listening on ocr.results");

    loop {
        match consumer.recv().await {
            Ok(msg) => {
                let Some(payload) = msg.payload() else { continue };
                match serde_json::from_slice::<OcrResult>(payload) {
                    Ok(result) => {
                        let id = match Uuid::parse_str(&result.job_id) {
                            Ok(id) => id,
                            Err(_) => {
                                tracing::warn!("bad job_id in result: {}", result.job_id);
                                continue;
                            }
                        };
                        // Mark complete and clear the stored image to reclaim space.
                        let res = sqlx::query(
                            "UPDATE jobs SET status = 'complete', result_text = $2, \
                             confidence = $3, image_b64 = NULL, updated_at = now() \
                             WHERE id = $1",
                        )
                        .bind(id)
                        .bind(&result.text)
                        .bind(result.confidence)
                        .execute(&pool)
                        .await;

                        match res {
                            Ok(r) if r.rows_affected() == 0 => {
                                tracing::warn!("result for unknown job {id}");
                            }
                            Ok(_) => tracing::info!("job {id} marked complete"),
                            Err(e) => tracing::error!("failed to update job {id}: {e}"),
                        }
                    }
                    Err(e) => tracing::warn!("failed to deserialise result: {e}"),
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
                .add_directive("job_service=debug".parse().unwrap()),
        )
        .init();

    let brokers =
        std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "localhost:9092".into());
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://chiron:chiron@localhost:5432/chiron".into());

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await
        .expect("failed to connect to Postgres");

    // Run the schema migration (idempotent — CREATE TABLE IF NOT EXISTS).
    let migration = include_str!("../migrations/001_init.sql");
    sqlx::raw_sql(migration)
        .execute(&pool)
        .await
        .expect("failed to run migration");
    tracing::info!("migration applied");

    let producer: FutureProducer = ClientConfig::new()
        .set("bootstrap.servers", &brokers)
        .set("message.max.bytes", "10485760")
        .set("compression.type", "lz4")
        .create()
        .expect("failed to create kafka producer");

    tokio::spawn(run_result_consumer(pool.clone(), brokers));

    let state = AppState { pool, producer };

    let app = Router::new()
        .route("/health", get(health))
        .route("/jobs", post(create_job))
        .route("/jobs/:job_id", get(get_job))
        .with_state(state);

    let addr = "0.0.0.0:8081";
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
