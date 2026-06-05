FROM python:3.12-slim
WORKDIR /app
RUN apt-get update && apt-get install -y libgomp1 && rm -rf /var/lib/apt/lists/*
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
# Pre-download EasyOCR model weights at build time so the pod starts fast
RUN python -c "import easyocr; easyocr.Reader(['en'], gpu=False)"
COPY worker.py .
CMD ["python", "worker.py"]
