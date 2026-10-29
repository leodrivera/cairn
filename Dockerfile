# syntax=docker/dockerfile:1

# Front-end: built once, embedded in the binary.
FROM node:24-alpine AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund --ignore-scripts
COPY web ./
COPY brand /src/brand
COPY tokens.css /src/tokens.css
RUN npm run build

# Binary: the Alpine host is musl, so the build is static for whichever architecture runs it.
FROM rust:1-alpine AS bin
RUN apk add --no-cache musl-dev ca-certificates
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY --from=web /src/web/dist ./web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked && cp target/release/cairn /cairn
RUN mkdir -p /empty/srv /empty/data

FROM scratch
COPY --from=bin /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=bin /cairn /cairn
COPY docker/passwd /etc/passwd
COPY --from=bin --chown=10001:10001 /empty/srv /srv
COPY --from=bin --chown=10001:10001 /empty/data /data
USER 10001:10001
ENV CAIRN_ROOT=/srv CAIRN_DATA=/data CAIRN_ADDR=0.0.0.0:8080
VOLUME ["/data"]
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s CMD ["/cairn", "health"]
ENTRYPOINT ["/cairn"]
CMD ["serve"]
