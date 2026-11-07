# Build from the repository root: docker build -f docker/pg-ext/Dockerfile -t lampl-pg-ext:test .
# Experimental image: Postgres 17 + pgvector + the lampl-pg-ext embedding worker. Not part of any release pipeline.
FROM public.ecr.aws/docker/library/rust:1-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c AS build
ARG PGVERSION=17
ARG PGVECTOR_TAG=v0.8.7
ARG PGRX_VERSION=0.19.3
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gnupg lsb-release \
    && curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc | gpg --dearmor -o /usr/share/keyrings/pgdg.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/pgdg.gpg] https://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" > /etc/apt/sources.list.d/pgdg.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends build-essential clang libclang-dev pkg-config libssl-dev git \
        postgresql-server-dev-${PGVERSION} \
    && rm -rf /var/lib/apt/lists/*

RUN git clone --depth 1 --branch ${PGVECTOR_TAG} https://github.com/pgvector/pgvector.git /pgvector \
    && make -C /pgvector OPTFLAGS="" \
    && make -C /pgvector install DESTDIR=/out

RUN cargo install cargo-pgrx --version =${PGRX_VERSION} --locked
RUN cargo pgrx init --pg${PGVERSION} /usr/lib/postgresql/${PGVERSION}/bin/pg_config

WORKDIR /src
COPY Cargo.toml rust-toolchain.toml ./
COPY crates/lampl-embed crates/lampl-embed
COPY crates/lampl-pg-ext crates/lampl-pg-ext
WORKDIR /src/crates/lampl-pg-ext
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/crates/lampl-pg-ext/target \
    cargo pgrx package --pg-config /usr/lib/postgresql/${PGVERSION}/bin/pg_config --out-dir /pkg \
    && cp -r /pkg/. /out/

FROM public.ecr.aws/docker/library/postgres:17-bookworm@sha256:665dc7b37ac52a17e298511414c21d23ffa4e6a6ef6225b3e5d59c7afb35e994
COPY --from=build /out/usr/ /usr/
COPY docker/pg-ext/init.sql /docker-entrypoint-initdb.d/00-lampl.sql
VOLUME /var/lib/lampl/models
CMD ["postgres", "-c", "shared_preload_libraries=lampl_pg_ext", "-c", "lampl.database=postgres", "-c", "lampl.model_dir=/var/lib/lampl/models"]
