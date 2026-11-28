# Same MinIO release as the previous probe; source build, not production image.
FROM golang:1.24.2-bookworm AS build
ENV GOTOOLCHAIN=local GOPROXY=https://proxy.golang.org GOSUMDB=sum.golang.org CGO_ENABLED=0 GOMAXPROCS=2
RUN go install -trimpath github.com/minio/minio@RELEASE.2025-04-22T22-12-26Z && go version -m /go/bin/minio
FROM debian:bookworm-slim
COPY --from=build /go/bin/minio /usr/local/bin/minio
ENV HOME=/tmp MINIO_BROWSER=off MINIO_UPDATE=off
ENTRYPOINT ["/usr/local/bin/minio"]
