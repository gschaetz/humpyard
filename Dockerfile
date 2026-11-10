# syntax=docker/dockerfile:1
# humpyard container image. Build:  docker build -t humpyard .
# Run:  docker run -p 8080:8080 -v ./config.toml:/etc/humpyard/config.toml:ro \
#         -v humpyard-data:/data -e GROQ_API_KEY ghcr.io/gschaetz/humpyard
# The config's `listen` must be 0.0.0.0:8080 and its ledger path /data/ledger.db (docs/deployment.md).

# The admin GUI (ADR 0032) is built to static files that the `ui` feature embeds in the binary.
FROM node:24-bookworm-slim AS gui
WORKDIR /gui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci --ignore-scripts
COPY ui/index.html ui/tsconfig.json ui/vite.config.ts ./
COPY ui/scripts ./scripts
COPY ui/src ./src
RUN npm run build

FROM rust:1.99-bookworm AS build
WORKDIR /src
# The image's own toolchain is used, so rust-toolchain.toml is deliberately not copied.
COPY Cargo.toml Cargo.lock ./
COPY build.rs ./
COPY src ./src
# The admin API's OpenAPI document is embedded in the binary (src/server/admin.rs).
COPY docs/openapi ./docs/openapi
COPY --from=gui /gui/dist ./ui/dist
RUN cargo build --release --locked --features ui
# /data is created here because the runtime image has no shell to do it.
RUN mkdir /data

# distroless/cc: glibc, CA certificates (the TLS verifier reads the system store), no shell.
FROM gcr.io/distroless/cc-debian12:nonroot
LABEL org.opencontainers.image.source="https://github.com/gschaetz/humpyard" \
      org.opencontainers.image.description="A gateway that sorts each LLM request onto the right model" \
      org.opencontainers.image.licenses="Apache-2.0"
COPY --from=build /src/target/release/humpyard /usr/local/bin/humpyard
COPY --from=build --chown=65532:65532 /data /data
VOLUME /data
EXPOSE 8080
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/humpyard"]
CMD ["serve", "--config", "/etc/humpyard/config.toml"]
