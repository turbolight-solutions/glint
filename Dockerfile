# Glint web service: the API, the built frontend and the embedded corpus in one stateless container.
#
# The corpus bundle is not in git. Build it first (see README) or take it from a release, and put it at
# corpus/embedded.bundle before running `docker build`.

FROM node:22-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.99-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY crates crates
COPY corpus/embedded.bundle corpus/embedded.bundle
RUN cargo build --release --locked -p glint-server -p glint-asn1

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/glint-server /usr/local/bin/glint-server
COPY --from=build /src/target/release/glint /usr/local/bin/glint
COPY --from=web /src/web/dist /app/web
ENV GLINT_BIND=0.0.0.0:8080 \
    GLINT_STATIC_DIR=/app/web
EXPOSE 8080
USER nonroot
ENTRYPOINT ["/usr/local/bin/glint-server"]
