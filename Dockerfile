FROM public.ecr.aws/docker/library/rust:1-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p lampl

FROM public.ecr.aws/docker/library/debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 10001 lampl
COPY --from=build /src/target/release/lampl /usr/local/bin/lampl
USER lampl
ENV HOME=/home/lampl
ENTRYPOINT ["lampl"]
CMD ["serve"]
