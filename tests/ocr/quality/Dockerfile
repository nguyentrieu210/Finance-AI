# Independent synthetic image quality/deskew compatibility probe.
FROM debian:bookworm-slim
RUN apt-get update -qq && apt-get install -y --no-install-recommends python3 python3-numpy python3-opencv python3-pil fonts-dejavu-core && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY quality.py test_quality.py /app/
USER 65534:65534
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 OMP_NUM_THREADS=1 OPENBLAS_NUM_THREADS=1
CMD ["python3", "-B", "-m", "unittest", "discover", "-s", "/app", "-p", "test_*.py", "-v"]
