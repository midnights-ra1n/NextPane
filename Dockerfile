# NextPane server image. Build: docker build -t nextpane .
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p nextpane-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --home-dir /data nextpane && mkdir /data && chown nextpane:nextpane /data
COPY --from=build /src/target/release/nextpane-server /usr/bin/nextpane-server
ENV NEXTPANE_DATA=/data NEXTPANE_LISTEN=0.0.0.0:3000 NEXTPANE_INSTALL=docker
VOLUME /data
EXPOSE 3000
USER nextpane
ENTRYPOINT ["/usr/bin/nextpane-server"]
