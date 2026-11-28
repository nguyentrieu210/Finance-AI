# Isolated synthetic PDF compatibility test image; not production worker.
FROM debian:bookworm-slim
RUN apt-get update -qq && apt-get install -y --no-install-recommends python3 poppler-utils && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY renderer.py test_renderer.py /app/
USER 65534:65534
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 LC_ALL=C.UTF-8
CMD ["python3", "-B", "-m", "unittest", "discover", "-s", "/app", "-p", "test_*.py", "-v"]
