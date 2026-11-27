# Synthetic CPU compatibility baseline, not a production image.
FROM python:3.11-slim-bookworm
RUN apt-get update -qq && apt-get install -y --no-install-recommends libgl1 libglib2.0-0 libgomp1 fonts-dejavu-core && rm -rf /var/lib/apt/lists/*
RUN pip install --no-cache-dir paddlepaddle==3.0.0 -i https://www.paddlepaddle.org.cn/packages/stable/cpu/
RUN pip install --no-cache-dir paddleocr==3.1.1 'paddlex[ocr]==3.1.1' 'numpy<2' 'pillow==11.2.1' 'langchain==0.3.27' 'langchain-community==0.3.27' 'langchain-core==0.3.76' 'langchain-openai==0.3.28' && pip check
WORKDIR /app
COPY prepare_models.py probe.py /app/
RUN python prepare_models.py
ENV HOME=/tmp PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK=True
USER 65534:65534
CMD ["python", "-B", "/app/probe.py"]
