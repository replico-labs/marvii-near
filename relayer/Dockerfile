# Build from the near/ directory: docker build -f relayer/Dockerfile .
# (the relayer uses near/core, which shares near/extension/src/evm.rs)
# Both stages on the same Debian release: a binary built against a newer
# glibc than the runtime image has fails to start.
FROM rust:1.97.1-slim-trixie AS build
WORKDIR /src
COPY core ./core
COPY extension/src/evm.rs ./extension/src/evm.rs
COPY relayer ./relayer
RUN cd relayer && cargo build --release --locked

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/relayer/target/release/marvii-near-relayer /usr/local/bin/
ENV BIND=0.0.0.0 PORT=8787
EXPOSE 8787
# Secrets come from the platform's environment, never the image:
# MARVII_RELAYER_SECRET_KEY, plus the public MARVII_RELAYER_ACCOUNT,
# MARVII_NEAR_EXTENSION, MARVII_NEAR_KNOWN_EXTENSIONS,
# MARVII_NEAR_WALLET_CODE_HASH and MARVII_PAGE_ORIGIN.
CMD ["marvii-near-relayer"]
