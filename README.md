# Chiron

An OCR pipeline built on Kafka and Kubernetes. A Vite/React frontend lets users upload images; a thin Rust API gateway forwards requests to a Rust job service, which owns Postgres (durable job state) and Kafka (job submission + result consumption); a Python worker consumes jobs, runs OCR via EasyOCR, and publishes results back.

```
Browser → [Frontend] → [API Gateway] → [Job Service] → Postgres (durable job state)
                       (edge proxy)          │   ▲
                              ocr.jobs (Kafka)│   │ ocr.results (Kafka)
                                              ▼   │
                                         [OCR Worker]
```

The frontend polls for results by job ID rather than holding an open connection. The gateway holds no state, and the job service persists every job in Postgres — so both can be scaled freely and the OCR worker scales independently behind Kafka.

---

## Services

| Service | Language / Runtime | Role |
|---|---|---|
| `frontend` | TypeScript · Vite · React | UI, image upload, job polling |
| `api-gateway` | Rust · Axum | Thin edge proxy in front of the job service |
| `job-service` | Rust · Axum · sqlx · rdkafka | Owns Postgres + Kafka (job producer, result consumer) |
| `ocr-worker` | Python · EasyOCR | Kafka consumer, OCR inference |

### Frontend (`services/frontend`)

Vite + React SPA. Uploads a base64-encoded image to `POST /api/ocr`, receives a `job_id`, then polls `GET /api/ocr/:job_id` at 1.5 s intervals until the result arrives. The Vite dev server proxies `/api/*` to the API gateway — no CORS config needed locally.

Key files:
- `src/api.ts` — typed fetch wrappers (`submitOcr`, `pollOcr`) plus client-side image compression (`compressAndEncode`)
- `src/useOcrJob.ts` — state machine hook: `idle → submitting → polling → complete`
- `src/App.tsx` — renders each phase of the state machine
- `nginx.conf` — used in the production Docker image; proxies `/api/` in-cluster

### API Gateway (`services/api-gateway`)

A thin, stateless edge proxy. It holds no job state of its own — it forwards each request to the job service and mirrors the upstream status and body back to the caller. Keeping this layer stateless lets it scale freely.

| Method | Path | Forwards to |
|---|---|---|
| `GET` | `/health` | (answered locally) |
| `POST` | `/ocr` | `POST {job-service}/jobs` |
| `GET` | `/ocr/:job_id` | `GET {job-service}/jobs/:job_id` |

Key dependencies: `axum`, `reqwest`, `tower-http`.

### Job Service (`services/job-service`)

Owns all stateful interaction — Postgres and Kafka. Exposes `POST /jobs` and `GET /jobs/:job_id` to the gateway.

- **Submit** (`POST /jobs`): transactionally inserts a `pending` row, publishes the job to `ocr.jobs`, then marks it `in_progress` and commits — so a job is never persisted as in-flight unless it actually reached Kafka.
- **Poll** (`GET /jobs/:job_id`): reads job state from Postgres, returning `pending`, `complete` (with `text`/`confidence`), or `failed` (with `error`).
- **Result consumer**: a background task consumes `ocr.results` and applies each result to Postgres with an idempotent `UPDATE`, clearing the stored image to reclaim space.

The schema (`migrations/001_init.sql`) is applied on startup (idempotent `CREATE TABLE IF NOT EXISTS`) and also via a dedicated Kubernetes migration Job.

Key dependencies: `axum`, `rdkafka`, `sqlx` (Postgres), `uuid`, `tower-http`.

### OCR Worker (`services/ocr-worker`)

Kafka consumer loop. Decodes the base64 image, runs `easyocr.Reader.readtext`, and publishes `{ job_id, text, confidence }` to `ocr.results`. On failure it reports the job as failed or routes the message to the DLQ rather than crashing (see [Kafka Topics](#kafka-topics)). EasyOCR model weights are baked into the Docker image at build time so pod/container startup isn't blocked on a download.

---

## Running Locally

**Prerequisites:** Docker Desktop (or Docker Engine + Compose plugin).

```bash
make dev
```

That's it. On first run Docker builds all four service images — the OCR worker build downloads EasyOCR model weights, which takes a few minutes. Subsequent starts are fast. Kafka and Postgres run as their own containers.

| Service | URL |
|---|---|
| Frontend (Vite dev server, hot reload) | http://localhost:5173 |
| API Gateway | http://localhost:8080 |
| Kafka broker | localhost:9092 |

### Makefile targets

```bash
make dev                    # build + start all services (detached)
make build                  # build images without starting
make down                   # stop and remove containers (keeps volumes)
make clean                  # full reset including volumes (re-runs npm install on next start)

make logs                   # tail logs for all services
make logs-gateway           # tail api-gateway only
make logs-worker            # tail ocr-worker only
make logs-frontend          # tail frontend only

make restart svc=ocr-worker # restart a single service
```

### Running the frontend outside Docker

If you want native Vite hot reload without the container overhead:

```bash
cd services/frontend
npm install
API_TARGET=http://localhost:8080 npm run dev
```

The `API_TARGET` env var tells `vite.config.ts` to proxy to your locally-running gateway instead of the Docker service name.

---

## Project Structure

```
chiron/
├── Cargo.toml                        # Rust workspace root
├── docker-compose.yml                # Local dev environment
├── Makefile                          # Dev + k8s convenience targets
├── services/
│   ├── api-gateway/                  # Rust / Axum — thin edge proxy
│   │   ├── Cargo.toml
│   │   ├── Dockerfile
│   │   └── src/main.rs
│   ├── job-service/                  # Rust / Axum — Postgres + Kafka owner
│   │   ├── Cargo.toml
│   │   ├── Dockerfile
│   │   ├── migrations/001_init.sql
│   │   └── src/main.rs
│   ├── ocr-worker/                   # Python
│   │   ├── worker.py
│   │   ├── requirements.txt
│   │   └── Dockerfile
│   └── frontend/                     # Vite + React + TypeScript
│       ├── src/
│       │   ├── App.tsx
│       │   ├── api.ts
│       │   ├── useOcrJob.ts
│       │   └── main.tsx
│       ├── index.html
│       ├── vite.config.ts
│       ├── nginx.conf                # Production image only
│       └── Dockerfile
└── k8s/
    ├── base/                         # Canonical manifests (Kustomize base)
    │   ├── kustomization.yaml
    │   ├── namespace.yaml
    │   ├── kafka/kafka.yaml          # Strimzi Kafka cluster + topics
    │   ├── postgres/statefulset.yaml
    │   ├── api-gateway/deployment.yaml
    │   ├── job-service/              # deployment + migration Job + SQL
    │   ├── ocr-worker/deployment.yaml
    │   └── frontend/deployment.yaml
    └── overlays/                     # Ring-specific overrides
        ├── local/                    # minikube / kind
        ├── dev/                      # dev cluster
        └── prod/                     # production cluster
```

---

## Deploying to Kubernetes

Manifests are managed with **Kustomize** (built into `kubectl`). A `base/` layer holds the canonical definitions; overlays patch only what differs per ring.

### What changes per ring

| | `local` | `dev` | `prod` |
|---|---|---|---|
| Namespace | `chiron-local` | `chiron-dev` | `chiron-prod` |
| Image tags | `:local` | `:dev` | `:1.0.0` (pinned semver) |
| Kafka | `host.docker.internal:9092` | `kafka-dev` cluster | `kafka-prod` cluster |
| Replicas | 1 each | 1 each | 2–3 + HPA |
| TLS | none | Let's Encrypt staging | Let's Encrypt prod |
| OCR worker RAM limit | 4 Gi (base) | 2 Gi | 8 Gi |

### First-time cluster setup

```bash
# 1. Create the Strimzi operator (manages the Kafka cluster CR)
kubectl create namespace kafka
kubectl apply -f https://strimzi.io/install/latest?namespace=kafka -n kafka

# 2. Apply a ring — Kustomize renders base + overlay and pipes to kubectl
kubectl apply -k k8s/overlays/local    # or dev / prod
```

### Makefile targets

```bash
make k8s-diff ring=local    # dry-run: print rendered manifests without applying
make k8s-apply ring=prod    # apply a ring to the active cluster context
```

### HPA (prod only)

The `prod` overlay adds a `HorizontalPodAutoscaler` for both `api-gateway` and `ocr-worker`. The OCR worker is capped at **6 replicas** to match the `ocr.jobs` topic partition count — additional replicas beyond the partition count would be assigned no work by Kafka.

---

## Kafka Topics

| Topic | Publisher | Consumer | Retention |
|---|---|---|---|
| `ocr.jobs` | `job-service` | `ocr-worker` | 1 hour |
| `ocr.results` | `ocr-worker` | `job-service` | 1 hour |
| `ocr.jobs.dlq` | `ocr-worker` | (manual inspection / replay) | 7 days |

The worker never lets a single bad message crash its consumer loop (which would leave the offset uncommitted and reprocess the poison message forever on restart). Failures are split by whether the job can be identified:

- **Unidentifiable** (malformed JSON, missing `job_id`) → routed to `ocr.jobs.dlq`, wrapping the original payload plus an `error` field for inspection/replay.
- **Identifiable but failed** (undecodable image, OCR error) → a failure result is published back on `ocr.results`, and `job-service` marks the job `failed`. The frontend stops polling on that terminal state instead of waiting forever.

### Delivery guarantee

The worker processes `ocr.jobs` **at-least-once**. It disables Kafka auto-commit and instead commits each source offset by hand only after the corresponding result (success, failure, or DLQ message) has been acknowledged by the broker. A crash anywhere between consuming a job and its result being delivered leaves the offset uncommitted, so the job is reprocessed on restart rather than silently dropped (stuck `in_progress`).

Reprocessing is safe because it can only produce a duplicate result: `job-service` applies results with an idempotent `UPDATE ... WHERE id = <job_id>`, so re-delivering a result just re-sets the same terminal state.

The `ocr.jobs` and `ocr.results` topics are created with 6 partitions to allow horizontal scaling of the OCR worker up to 6 parallel consumers within a single consumer group. The DLQ uses a single partition since it carries only occasional failures.
