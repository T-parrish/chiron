.PHONY: dev build down logs restart clean \
        logs-gateway logs-worker logs-frontend logs-kafka

# ---------------------------------------------------------------------------
# Local dev
# ---------------------------------------------------------------------------

## Start all services (build if needed, then up in detached mode)
dev:
	docker compose up --build -d
	@echo ""
	@echo "  Frontend  → http://localhost:5173"
	@echo "  API       → http://localhost:8080"
	@echo "  Kafka     → localhost:9092"
	@echo ""
	@echo "  Run 'make logs' to tail all services."

## Build all images without starting
build:
	docker compose build

## Stop and remove containers (keeps volumes)
down:
	docker compose down

## Stop, remove containers AND volumes (full reset incl. node_modules cache)
clean:
	docker compose down -v

## Restart a single service, e.g.: make restart svc=ocr-worker
restart:
	docker compose restart $(svc)

# ---------------------------------------------------------------------------
# Logs
# ---------------------------------------------------------------------------

## Tail logs for all services
logs:
	docker compose logs -f

logs-gateway:
	docker compose logs -f api-gateway

logs-worker:
	docker compose logs -f ocr-worker

logs-frontend:
	docker compose logs -f frontend

logs-kafka:
	docker compose logs -f kafka

# ---------------------------------------------------------------------------
# Kubernetes (Kustomize)
# ---------------------------------------------------------------------------

## Dry-run: print rendered manifests for a ring (ring=local|dev|prod)
k8s-diff:
	kubectl kustomize k8s/overlays/$(ring)

## Apply manifests for a ring (ring=local|dev|prod)
k8s-apply:
	kubectl apply -k k8s/overlays/$(ring)
