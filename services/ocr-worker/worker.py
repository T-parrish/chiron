import base64
import json
import logging
import os
import io

from confluent_kafka import Consumer, Producer, KafkaException
import easyocr

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger(__name__)

BROKERS = os.environ.get("KAFKA_BROKERS", "localhost:9092")
GROUP_ID = os.environ.get("KAFKA_GROUP_ID", "ocr-worker")

reader = easyocr.Reader(["en"], gpu=False)

consumer = Consumer({
    "bootstrap.servers": BROKERS,
    "group.id": GROUP_ID,
    "auto.offset.reset": "earliest",
})
producer = Producer({"bootstrap.servers": BROKERS})

consumer.subscribe(["ocr.jobs"])
log.info("OCR worker started, waiting for jobs…")

try:
    while True:
        msg = consumer.poll(timeout=1.0)
        if msg is None:
            continue
        if msg.error():
            raise KafkaException(msg.error())

        job = json.loads(msg.value())
        job_id = job["job_id"]
        image_bytes = base64.b64decode(job["image_b64"])

        log.info("processing job %s", job_id)
        results = reader.readtext(image_bytes, detail=1)

        combined_text = " ".join(r[1] for r in results)
        avg_confidence = (
            sum(r[2] for r in results) / len(results) if results else 0.0
        )

        result_payload = json.dumps({
            "job_id": job_id,
            "text": combined_text,
            "confidence": round(avg_confidence, 4),
        }).encode()

        producer.produce("ocr.results", key=job_id, value=result_payload)
        producer.poll(0)
        log.info("job %s complete: %d chars", job_id, len(combined_text))
except KeyboardInterrupt:
    pass
finally:
    consumer.close()
