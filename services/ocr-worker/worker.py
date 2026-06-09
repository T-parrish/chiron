import base64
import json
import logging
import os

from confluent_kafka import Consumer, Producer, KafkaException
import easyocr

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger(__name__)

BROKERS = os.environ.get("KAFKA_BROKERS", "localhost:9092")
GROUP_ID = os.environ.get("KAFKA_GROUP_ID", "ocr-worker")
JOBS_TOPIC = os.environ.get("KAFKA_JOBS_TOPIC", "ocr.jobs")
RESULTS_TOPIC = os.environ.get("KAFKA_RESULTS_TOPIC", "ocr.results")
# Messages we can't even identify (bad JSON, missing job_id) are routed here so
# they don't crash the loop and get reprocessed forever. Failures for messages
# we CAN identify are reported back on RESULTS_TOPIC and mark the job failed.
DLQ_TOPIC = os.environ.get("KAFKA_DLQ_TOPIC", "ocr.jobs.dlq")

reader = easyocr.Reader(["en"], gpu=False)

consumer = Consumer({
    "bootstrap.servers": BROKERS,
    "group.id": GROUP_ID,
    "auto.offset.reset": "earliest",
    # At-least-once: we commit each offset by hand only after its result has been
    # delivered to Kafka. Auto-commit could advance the offset before the result
    # is produced, silently dropping the job if the worker then crashed.
    "enable.auto.commit": False,
})
producer = Producer({"bootstrap.servers": BROKERS})

consumer.subscribe([JOBS_TOPIC])


def run_ocr(image_b64: str) -> tuple[str, float]:
    image_bytes = base64.b64decode(image_b64)
    results = reader.readtext(image_bytes, detail=1)
    combined_text = " ".join(r[1] for r in results)
    avg_confidence = sum(r[2] for r in results) / len(results) if results else 0.0
    return combined_text, round(avg_confidence, 4)


def publish_success(job_id: str, text: str, confidence: float) -> None:
    payload = json.dumps({
        "job_id": job_id,
        "text": text,
        "confidence": confidence,
    }).encode()
    producer.produce(RESULTS_TOPIC, key=job_id, value=payload)
    producer.poll(0)


def publish_failure(job_id: str, error: Exception) -> None:
    payload = json.dumps({
        "job_id": job_id,
        "error": f"{type(error).__name__}: {error}",
    }).encode()
    producer.produce(RESULTS_TOPIC, key=job_id, value=payload)
    producer.poll(0)


def send_to_dlq(msg, error: Exception) -> None:
    raw = msg.value()
    envelope = json.dumps({
        "error": f"{type(error).__name__}: {error}",
        "original": raw.decode("utf-8", errors="replace") if raw else None,
    }).encode()
    # Preserve the original key so a replay tool can still correlate by job_id.
    producer.produce(DLQ_TOPIC, key=msg.key(), value=envelope)
    producer.poll(0)


def handle(msg) -> None:
    # Phase 1 — identify the job. If we can't extract a job_id there's nothing to
    # mark failed, so the message goes to the DLQ.
    try:
        job = json.loads(msg.value())
        job_id = job["job_id"]
        image_b64 = job["image_b64"]
    except Exception as error:
        log.exception("unparseable job message; routing to DLQ")
        send_to_dlq(msg, error)
        return

    # Phase 2 — run OCR. A failure here belongs to a known job, so report it back
    # as a failed result (a terminal state the frontend stops polling on) rather
    # than leaving the job stuck in_progress forever.
    log.info("processing job %s", job_id)
    try:
        text, confidence = run_ocr(image_b64)
    except Exception as error:
        log.exception("OCR failed for job %s; publishing failure", job_id)
        publish_failure(job_id, error)
        return

    publish_success(job_id, text, confidence)
    log.info("job %s complete: %d chars", job_id, len(text))


log.info("OCR worker started, waiting for jobs…")

try:
    while True:
        msg = consumer.poll(timeout=1.0)
        if msg is None:
            continue
        if msg.error():
            # Transport-level error (e.g. partition EOF) — not a poison message.
            # Log and keep polling rather than tearing down the worker.
            log.warning("kafka message error: %s", msg.error())
            continue

        try:
            handle(msg)
        except Exception as error:
            # handle() resolves its own failures; this is a last-resort guard so
            # an unexpected error (e.g. the Kafka producer raising) can't kill
            # the loop and trigger a reprocessing crash-loop.
            log.exception("unexpected error handling message; routing to DLQ")
            try:
                send_to_dlq(msg, error)
            except Exception:
                log.exception("failed to publish message to DLQ")

        # Block until the result/failure/DLQ message this iteration produced is
        # acknowledged by the broker. Only then is it safe to commit the source
        # offset. If delivery didn't complete, leave the offset uncommitted so
        # the message is reprocessed — job-service UPDATEs are keyed by job_id
        # and idempotent, so a duplicate result is harmless.
        if producer.flush(timeout=10) > 0:
            log.error("producer flush incomplete; not committing offset, will reprocess")
            continue
        try:
            consumer.commit(message=msg, asynchronous=False)
        except Exception:
            log.exception("failed to commit offset; message will be reprocessed")
except KeyboardInterrupt:
    pass
finally:
    producer.flush()
    consumer.close()
