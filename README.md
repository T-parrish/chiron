# Chiron

An OCR pipeline built on Kafka and Kubernetes. A Vite/React frontend lets users upload images; a Rust API gateway submits jobs to Kafka and exposes a polling endpoint; a Python worker consumes jobs, runs OCR via EasyOCR, and publishes results back.

```
Browser → [Frontend] → [Rust API Gateway] → Kafka (ocr.jobs)
                             ↑                      ↓
                      Kafka (ocr.results)   [Python OCR Worker]
```

The frontend polls for results by job ID rather than holding an open connection, which keeps the gateway stateless and lets the OCR worker scale independently.

---

## Services

| Service | Language / Runtime | Role |
|---|---|---|
| `frontend` | TypeScript · Vite · React | UI, image upload, job polling |
| `api-gateway` | Rust · Axum | HTTP API, Kafka producer/consumer |
| `ocr-worker` | Python · EasyOCR | Kafka consumer, OCR inference |

### Frontend (`services/frontend`)

Vite + React SPA. Uploads a base64-encoded image to `POST /api/ocr`, receives a `job_id`, then polls `GET /api/ocr/:job_id` at 1.5 s intervals until the result arrives. The Vite dev server proxies `/api/*` to the API gateway — no CORS config needed locally.

Key files:
- `src/api.ts` — typed fetch wrappers (`submitOcr`, `pollOcr`, `toBase64`)
- `src/useOcrJob.ts` — state machine hook: `idle → submitting → polling → complete`
- `src/App.tsx` — renders each phase of the state machine
- `nginx.conf` — used in the production Docker image; proxies `/api/` in-cluster

### API Gateway (`services/api-gateway`)

Axum HTTP server with three routes:

| Method | Path | Description |
|---|---|---|
| `GET` | `/health` | Liveness/readiness probe |
| `POST` | `/ocr` | Accepts `{ image_b64 }`, publishes to `ocr.jobs`, returns `{ job_id }` |
| `GET` | `/ocr/:job_id` | Returns `{ status: "pending" }` or `{ status: "complete", text, confidence }` |

A background task consumes `ocr.results` and resolves waiting poll requests via a `DashMap` of in-flight job IDs.

Key dependencies: `axum`, `rdkafka`, `dashmap`, `uuid`, `tower-http`.

### OCR Worker (`services/ocr-worker`)

Simple Kafka consumer loop. Decodes the base64 image, runs `easyocr.Reader.readtext`, and publishes `{ job_id, text, confidence }` to `ocr.results`. EasyOCR model weights are baked into the Docker image at build time so pod/container startup isn't blocked on a download.

---

## Running Locally

**Prerequisites:** Docker Desktop (or Docker Engine + Compose plugin).

```bash
make dev
```

That's it. On first run Docker builds all three images — the OCR worker build downloads EasyOCR model weights, which takes a few minutes. Subsequent starts are fast.

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
│   ├── api-gateway/                  # Rust / Axum
│   │   ├── Cargo.toml
│   │   ├── Dockerfile
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
    │   ├── api-gateway/deployment.yaml
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
| `ocr.jobs` | `api-gateway` | `ocr-worker` | 1 hour |
| `ocr.results` | `ocr-worker` | `api-gateway` | 1 hour |
| `ocr.jobs.dlq` | `ocr-worker` | (manual inspection / replay) | 7 days |

The worker never lets a single bad message crash its consumer loop (which would leave the offset uncommitted and reprocess the poison message forever on restart). Failures are split by whether the job can be identified:

- **Unidentifiable** (malformed JSON, missing `job_id`) → routed to `ocr.jobs.dlq`, wrapping the original payload plus an `error` field for inspection/replay.
- **Identifiable but failed** (undecodable image, OCR error) → a failure result is published back on `ocr.results`, and `job-service` marks the job `failed`. The frontend stops polling on that terminal state instead of waiting forever.

Both topics are created with 6 partitions to allow horizontal scaling of the OCR worker up to 6 parallel consumers within a single consumer group.
