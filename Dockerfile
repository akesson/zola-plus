FROM rust:slim-bookworm AS builder

ARG USE_GH_RELEASE=false
ARG ZOLA_RELEASE_VERSION=latest
RUN apt-get update -y && \
  apt-get install -y pkg-config make g++ libssl-dev curl jq tar gzip

WORKDIR /app
COPY . .

RUN if [ "${USE_GH_RELEASE}" = "true" ]; then \
    if [ "${ZOLA_RELEASE_VERSION}" = "latest" ]; then \
      export ZOLA_VERSION=$(curl -sL https://api.github.com/repos/akesson/zola-plus/releases/latest | jq -r .tag_name); \
    else \
      export ZOLA_VERSION="${ZOLA_RELEASE_VERSION}"; \
    fi && \
    curl -sL --fail --output zola-plus.tar.gz https://github.com/akesson/zola-plus/releases/download/${ZOLA_VERSION}/zola-plus-${ZOLA_VERSION}-$(uname -m)-unknown-linux-gnu.tar.gz && \
    tar -xzvf zola-plus.tar.gz zola-plus; \
  else \
    cargo build --release && \
    cp target/$(uname -m)-unknown-linux-gnu/release/zola-plus .; \
  fi && ./zola-plus --version

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /app/zola-plus /bin/zola-plus
ENTRYPOINT [ "/bin/zola-plus" ]
