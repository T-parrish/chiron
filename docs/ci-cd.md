# CI/CD: Vultr Container Registry → Vultr Kubernetes Engine

The pipeline ([.github/workflows/ci-cd.yml](../.github/workflows/ci-cd.yml)) builds
all four service images, pushes them to the Vultr Container Registry (VCR) tagged
with the immutable git SHA, and rolls them out to Vultr Kubernetes Engine (VKE)
with Kustomize.

```
push main   →  build+push images  →  deploy to chiron-dev
push tag v* →  build+push images  →  deploy to chiron-prod
```

## One-time setup

### 1. Create the registry and cluster (Vultr console)

- **Container Registry** — note the URL (`<region>.vultrcr.com/<name>`, e.g.
  `sjc.vultrcr.com/chiron`) and the robot/API credentials.
- **Kubernetes Engine cluster** — download its kubeconfig.

Then update the registry host/name in both overlays if yours differs from the
`sjc.vultrcr.com/chiron` placeholder:
`k8s/overlays/dev/kustomization.yaml`, `k8s/overlays/prod/kustomization.yaml`.

### 2. Create the image-pull secret in each namespace

The Deployments reference `imagePullSecrets: [vcr-credentials]` (added via a patch
in the dev/prod overlays). Create it once per namespace, against each cluster:

```sh
kubectl create namespace chiron-dev   # if not already created by the overlay
kubectl -n chiron-dev create secret docker-registry vcr-credentials \
  --docker-server=sjc.vultrcr.com/chiron \
  --docker-username=<VULTR_REGISTRY_USER> \
  --docker-password=<VULTR_REGISTRY_PASS>
# repeat with chiron-prod against the prod cluster
```

### 3. Add GitHub Actions secrets

`Settings → Secrets and variables → Actions`:

| Secret                 | Value                                              |
| ---------------------- | -------------------------------------------------- |
| `VULTR_REGISTRY_URL`   | `sjc.vultrcr.com/chiron` (host + name, no scheme)  |
| `VULTR_REGISTRY_USER`  | registry username                                  |
| `VULTR_REGISTRY_PASS`  | registry password / API key                        |
| `KUBECONFIG_DEV`       | `base64 -w0 < dev-kubeconfig.yaml`                 |
| `KUBECONFIG_PROD`      | `base64 -w0 < prod-kubeconfig.yaml`                |

(On macOS, `base64 < file | tr -d '\n'` — there's no `-w` flag.)

## How a deploy works

1. Each image is pushed as `…/<service>:<git-sha>` (immutable) plus a moving
   `…/<service>:<branch-or-tag>` convenience tag.
2. The deploy job runs `kustomize edit set image chiron/<svc>=…:<git-sha>` in the
   target overlay, so the rollout is pinned to the exact commit.
3. `db-migrate` is deleted before apply (immutable Job spec; migration is
   idempotent and also runs on job-service startup), then `kubectl apply -k`.
4. `kubectl rollout status` gates the job on a healthy rollout.

## Rolling back

Re-run the deploy with an older SHA, or directly:

```sh
kubectl -n chiron-prod rollout undo deploy/api-gateway
```

## Notes / next steps

- **GitOps alternative.** Because everything is already Kustomize overlays, you
  could drop the cluster-credentials-in-CI model and have Argo CD or Flux pull
  changes instead. CI would only build/push and commit the SHA bump; the
  in-cluster controller applies it.
- **Build cache.** The Rust images use cargo-chef + BuildKit's GHA cache
  (`cache-to: type=gha`), so unchanged dependencies are not recompiled across
  runs. The first run is slow (cold cache); subsequent runs are fast.
