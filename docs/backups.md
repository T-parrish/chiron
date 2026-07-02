# Postgres backups

A `postgres-backup` CronJob ([k8s/base/postgres-backup/cronjob.yaml](../k8s/base/postgres-backup/cronjob.yaml))
runs nightly at 03:00 UTC in the `dev` and `prod` rings (not `local`). Each run:

1. `pg_dump -Fc` of the chiron database (compressed custom format) into an
   `emptyDir` scratch volume.
2. Uploads it to S3-compatible object storage (Vultr Object Storage) as
   `s3://<bucket>/postgres/chiron-<UTC timestamp>.dump`.
3. Prunes objects older than `RETENTION_DAYS` (14). Pruning only runs after a
   successful upload, so consecutive failed backups can never age out the last
   good one.

The PVC behind the Postgres StatefulSet survives node loss, but not PVC/namespace
deletion or bad data — this CronJob is what makes the data actually recoverable.

## One-time setup (per ring)

### 1. Create the bucket and keys (Vultr console)

Object Storage → create (or reuse) a subscription, note the S3 endpoint
(e.g. `sjc1.vultrobjects.com`), create a bucket per ring
(e.g. `chiron-backups-dev`, `chiron-backups-prod`), and note the access/secret
keys.

### 2. Create the `backup-credentials` secret

```sh
kubectl -n chiron-prod create secret generic backup-credentials \
  --from-literal=AWS_ACCESS_KEY_ID=<access-key> \
  --from-literal=AWS_SECRET_ACCESS_KEY=<secret-key> \
  --from-literal=S3_ENDPOINT=https://sjc1.vultrobjects.com \
  --from-literal=S3_BUCKET=chiron-backups-prod
# repeat with chiron-dev / chiron-backups-dev against the dev cluster
```

The Postgres connection string comes from the existing `postgres-credentials`
secret; nothing extra to configure.

## Verifying

Trigger a run by hand rather than waiting for 03:00:

```sh
kubectl -n chiron-prod create job --from=cronjob/postgres-backup backup-manual-$(date +%s)
kubectl -n chiron-prod logs -l app=postgres-backup -f
```

Then confirm the object exists:

```sh
aws s3 ls s3://chiron-backups-prod/postgres/ --endpoint-url https://sjc1.vultrobjects.com
```

## Restoring

Download the dump you want, then run `pg_restore` from a throwaway pod in the
cluster (it has network access to `postgres` and the right client version):

```sh
# 1. Pick a backup
aws s3 ls s3://chiron-backups-prod/postgres/ --endpoint-url https://sjc1.vultrobjects.com

# 2. Start a restore pod with the dump available
kubectl -n chiron-prod run pg-restore --rm -it --image=postgres:16-alpine \
  --env-from=secretRef/postgres-credentials --command -- sh
# (--env-from requires kubectl >= 1.28; otherwise create a pod manifest that
#  uses envFrom: secretRef: postgres-credentials)

# 3. Inside the pod: fetch and restore
apk add --no-cache aws-cli
aws s3 cp s3://chiron-backups-prod/postgres/chiron-<ts>.dump /tmp/restore.dump \
  --endpoint-url https://sjc1.vultrobjects.com \
  # export AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY first (values from backup-credentials)

# --clean --if-exists drops and recreates objects, so this overwrites the
# current state of the database. Scale job-service down first if you need a
# consistent cutover:  kubectl scale deploy/job-service --replicas=0
pg_restore --clean --if-exists --no-owner -d "$DATABASE_URL" /tmp/restore.dump
```

Rehearse this once against the dev ring before you need it in prod — an
untested restore procedure is not a backup strategy.

## Tuning

- **Schedule / retention**: edit `schedule:` and the `RETENTION_DAYS` env in
  the CronJob. 14 daily dumps of this database are small (job rows only; the
  result consumer clears stored images), so retention is cheap to raise.
- **Point-in-time recovery**: nightly logical dumps mean up to 24 h of data
  loss. If that ever matters, the step up is WAL archiving via a Postgres
  operator (CloudNativePG has this built in) or Vultr Managed Postgres — not
  more cron frequency.
