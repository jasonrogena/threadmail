# Binary is static and bundles its own TLS roots (webpki-roots), so scratch works.
FROM scratch
ARG TARGETARCH
COPY dist/threadmail-linux-${TARGETARCH} /threadmail
ENTRYPOINT ["/threadmail"]
CMD ["serve"]
