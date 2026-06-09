-- Job state table. One row per OCR job.
--
-- status lifecycle:
--   pending      -> row inserted, not yet published to Kafka
--   in_progress  -> published to ocr.jobs, awaiting worker result
--   complete     -> worker result received; result_text/confidence populated
--   failed       -> worker reported an error (or publish failed before commit)
CREATE TABLE IF NOT EXISTS jobs (
    id          UUID PRIMARY KEY,
    status      TEXT        NOT NULL DEFAULT 'pending',
    image_b64   TEXT,                       -- cleared to NULL once complete
    result_text TEXT,
    confidence  REAL,
    error       TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS jobs_status_idx ON jobs (status);
