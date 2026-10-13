# Dependency review — 2026-09-27

The frontend audit is clear after updating the locked `devalue` dependency. Rust upgrades include `bytes`, `time`, `plist`/`quick-xml`, `anyhow`, `rand`, `rumqttc`, and `rustls`.

## Narrow certificate-library exceptions

`rumqttc 0.25.1` depends directly on `rustls-webpki 0.102.8`, but its only source reference is the `webpki::Error` payload of `tls::Error::WebPki`. It does not use that version for certificate parsing or validation. Its TLS connector uses the Rustls types re-exported by `tokio-rustls 0.26`; both mqx and that connector use patched `rustls 0.23.45` and `rustls-webpki 0.103.14` for certificate verification.

The `.cargo/audit.toml` exceptions cover only these four advisories against the unused old certificate-verification implementation:

- RUSTSEC-2026-0104: CRL parsing panic.
- RUSTSEC-2026-0098: URI name constraints.
- RUSTSEC-2026-0099: wildcard name constraints.
- RUSTSEC-2026-0049: CRL distribution-point matching.

Source reviewed: [rumqttc 0.25.1 TLS connector](https://docs.rs/crate/rumqttc/0.25.1/source/src/tls.rs). The exceptions must be removed or revalidated when the MQTT/TLS dependency graph changes. CI's `scripts/check-audit-exceptions.mjs` verifies the exact reviewed dependency version and that the old library remains an error-type-only dependency, so broadening its usage fails the check.

## Upstream maintenance warnings

Cargo audit also reports upstream unmaintained packages in the Tauri/GTK and MQTT dependency trees. These remain visible. It reports informational unsoundness warnings for GTK's `glib 0.18.5` variant-string iterator and the old `rand 0.7.3` implementation pulled through Tauri's parsing dependencies. mqx does not call the GLib iterator API or configure a custom logger that recursively calls rand. These are residual upstream risks; they are not silently suppressed or described as fixed. Revisit them when Tauri's GTK/dependency stack updates.

The automated audit checks published dependency advisories. It is not a full security assessment of mqx or proof that dependencies have no vulnerabilities.
