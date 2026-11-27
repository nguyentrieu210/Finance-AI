# Synthetic office compatibility, not an approved production converter.
FROM debian:bookworm-slim
RUN apt-get update -qq && apt-get install -y --no-install-recommends python3 python3-docx python3-openpyxl libreoffice-writer libreoffice-calc poppler-utils fonts-dejavu-core && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY probe.py office_guard.py /app/
ENV HOME=/tmp PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 LC_ALL=C.UTF-8
USER 65534:65534
CMD ["python3","-B","/app/probe.py"]
