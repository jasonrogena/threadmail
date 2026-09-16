# Built from pre-compiled static musl binaries (see
# .github/workflows/release.yml), not from source, so this has no Rust
# toolchain stage. `FROM scratch` works because the binary has zero runtime
# dependencies: it's statically linked and TLS root certs are compiled in
# via webpki-roots, not read from a host CA bundle.
FROM scratch
ARG TARGETARCH
COPY dist/threadmail-linux-${TARGETARCH} /threadmail
ENTRYPOINT ["/threadmail"]
CMD ["serve"]
