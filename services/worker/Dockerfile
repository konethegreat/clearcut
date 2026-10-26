# Written by Kone Tshivhinda & Claude
# The document worker: reads untrusted files. Run it with no internet and no credentials
# (the compose file puts it on an internal network).
# Build from the repository root:  docker build -f services/worker/Dockerfile -t clearcut-worker services/worker
FROM python:3.12-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends \
        tesseract-ocr tesseract-ocr-eng tesseract-ocr-afr \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10002 --no-create-home --shell /usr/sbin/nologin worker
WORKDIR /app
COPY requirements.txt constraints.txt ./
RUN pip install --no-cache-dir -r requirements.txt -c constraints.txt
COPY src ./src
USER worker
ENV PYTHONPATH=/app/src CLEARCUT_WORKER_HOST=0.0.0.0 CLEARCUT_WORKER_PORT=8090 CLEARCUT_WORKER_TESSERACT=tesseract
EXPOSE 8090
CMD ["python", "-m", "clearcut_worker"]
