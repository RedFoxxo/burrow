# burrow in a container: a static Rust build on Alpine. Directories such as
# Glama build this image, start it and read tools/list; burrow signs in only
# on the first tool call, so the placeholder LEAFWIKI_URL below is enough for
# that. To use it, set your own:
#
#   docker run -i --rm -e LEAFWIKI_URL=https://wiki.example.com \
#     -e LEAFWIKI_USERNAME -e LEAFWIKI_PASSWORD burrow
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM alpine:3
# rustls checks the wiki's certificate against the system's CA store.
RUN apk add --no-cache ca-certificates
COPY --from=build /src/target/release/burrow /usr/local/bin/burrow
ENV LEAFWIKI_URL=https://wiki.example.com
USER nobody
ENTRYPOINT ["burrow"]
