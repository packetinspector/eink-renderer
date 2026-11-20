# syntax=docker/dockerfile:1
# eink-renderer: Rust supervisor + pinned Chrome for Testing chrome-headless-shell (linux amd64 and arm64).

ARG CHROME_VERSION=158.0.8127.36
# sha256 of chrome-headless-shell-linux64.zip / chrome-headless-shell-linux-arm64.zip for CHROME_VERSION
ARG CHROME_SHA256_AMD64=2d3a23017323c54fd457a678e4351f895c4076d8050e39d5aa6247f93448b73a
ARG CHROME_SHA256_ARM64=176ee5c55486c22de43128871189061365ea28e881fc96016fd18e7cb59437c8

# ---- build the Rust binary on the build host, cross-compiling when the target differs ----
FROM --platform=$BUILDPLATFORM rust:1.100.0-trixie AS build
ARG TARGETARCH
ARG BUILDARCH
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) triple=x86_64-unknown-linux-gnu; gcc="gcc-x86-64-linux-gnu libc6-dev-amd64-cross" ;; \
      arm64) triple=aarch64-unknown-linux-gnu; gcc="gcc-aarch64-linux-gnu libc6-dev-arm64-cross" ;; \
      *) echo "unsupported TARGETARCH $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    if [ "$TARGETARCH" != "$BUILDARCH" ]; then \
      apt-get update && apt-get install -y --no-install-recommends $gcc && rm -rf /var/lib/apt/lists/*; \
    fi; \
    rustup target add "$triple"; echo "$triple" > /triple
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=eink-target-${TARGETARCH} \
    set -eu; t=$(cat /triple); \
    if [ "$TARGETARCH" = "$BUILDARCH" ]; then unset CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER; fi; \
    cargo build --release --locked --target "$t" && cp "target/$t/release/eink-renderer" /eink-renderer

# ---- fetch and verify the browser (no emulation needed) ----
FROM --platform=$BUILDPLATFORM debian:trixie-slim AS chrome
ARG TARGETARCH
ARG CHROME_VERSION
ARG CHROME_SHA256_AMD64
ARG CHROME_SHA256_ARM64
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates unzip && rm -rf /var/lib/apt/lists/*
RUN set -eu; \
    case "$TARGETARCH" in \
      amd64) plat=linux64; sum=$CHROME_SHA256_AMD64 ;; \
      arm64) plat=linux-arm64; sum=$CHROME_SHA256_ARM64 ;; \
    esac; \
    curl -fsSL -o /tmp/chs.zip "https://storage.googleapis.com/chrome-for-testing-public/${CHROME_VERSION}/${plat}/chrome-headless-shell-${plat}.zip"; \
    echo "${sum}  /tmp/chs.zip" | sha256sum -c -; \
    unzip -q /tmp/chs.zip -d /tmp; mv "/tmp/chrome-headless-shell-${plat}" /opt/chrome; \
    rm -f /opt/chrome/libvk_swiftshader.so /opt/chrome/vk_swiftshader_icd.json /opt/chrome/libvulkan.so.1; \
    find /opt/chrome/locales -type f ! -name 'en-US.pak' -delete

# ---- runtime ----
FROM debian:trixie-slim
ARG CHROME_VERSION
LABEL org.opencontainers.image.title="eink-renderer" \
      org.opencontainers.image.description="Home Assistant dashboard -> 1-bit PNG for e-paper displays" \
      io.github.eink-renderer.chrome-version="${CHROME_VERSION}"
RUN apt-get update && apt-get install -y --no-install-recommends \
      libglib2.0-0t64 libnspr4 libnss3 libatk1.0-0t64 libatk-bridge2.0-0t64 libdbus-1-3 \
      libx11-6 libxcomposite1 libxdamage1 libxext6 libxfixes3 libxrandr2 libgbm1 libexpat1 \
      libxcb1 libxkbcommon0 libasound2t64 libatspi2.0-0t64 \
      fontconfig fonts-dejavu-core tzdata ca-certificates tini \
    && rm -rf /var/lib/apt/lists/* /usr/share/doc /usr/share/man
COPY --from=chrome /opt/chrome /opt/chrome
COPY --from=build /eink-renderer /usr/local/bin/eink-renderer
# /data holds the pairing with Home Assistant and the last image; owned by the runtime user so a named
# volume mounted there (-v eink-data:/data) is writable without extra steps.
RUN mkdir -p /data && chown 1000:1000 /data && chmod 700 /data
USER 1000:1000
ENV HOME=/tmp
EXPOSE 10001
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 CMD ["/usr/local/bin/eink-renderer", "--healthcheck"]
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/eink-renderer"]
