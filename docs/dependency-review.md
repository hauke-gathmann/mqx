# Dependency review — 2026-09-27

The frontend audit is clear after updating the locked `devalue` dependency. Rust upgrades include `bytes`, `time`, `plist`/`quick-xml`, `anyhow`, `rand`, `rumqttc`, and `rustls`.

## Frontend audit update — 2026-10-30

Vite is updated to 8.2.4 to address later development-server advisories. The npm audit still reports a low-severity [deepmerge advisory](https://github.com/advisories/GHSA-gcqx-w48m-w2h4) and its dependent Svelte Vite plugin; no patched deepmerge version is available. These are build-time dependencies, not code shipped in the desktop app. Revisit the advisory when an upstream fix is released. CI fails on high or critical npm advisories and continues to report lower-severity findings.

## Narrow certificate-library exceptions

`rumqttc 0.25.1` depends directly on `rustls-webpki 0.102.8`, but its only source reference is the `webpki::Error` payload of `tls::Error::WebPki`. It does not use that version for certificate parsing or validation. Its TLS connector uses the Rustls types re-exported by `tokio-rustls 0.26`; both mqx and that connector use patched `rustls 0.23.46` and `rustls-webpki 0.103.16` for certificate verification.

The `.cargo/audit.toml` exceptions cover these advisories against the unused old certificate-verification implementation:

- RUSTSEC-2026-0104: CRL parsing panic.
- RUSTSEC-2026-0098: URI name constraints.
- RUSTSEC-2026-0099: wildcard name constraints.
- RUSTSEC-2026-0049: CRL distribution-point matching.
- RUSTSEC-2026-0382: distrusted certificate intermediates.

Source reviewed: [rumqttc 0.25.1 TLS connector](https://docs.rs/crate/rumqttc/0.25.1/source/src/tls.rs). The exceptions must be removed or revalidated when the MQTT/TLS dependency graph changes. CI's `scripts/check-audit-exceptions.mjs` verifies the exact reviewed dependency version and that the old library remains an error-type-only dependency, so broadening its usage fails the check.

For 0.2.1 the active TLS verifier is updated to rustls 0.23.46 and rustls-webpki 0.103.16, with updated webpki trust roots and root certificates. The audit guard also requires the active webpki 0.103 version to be at least 0.103.16, so the legacy exception cannot hide a vulnerable active verifier. The Tauri updater is updated to 2.14.1, which uses the patched ZIP 9 series on Windows.

## Upstream maintenance warnings

Cargo audit also reports upstream unmaintained packages in the Tauri/GTK and MQTT dependency trees. These remain visible. It reports informational unsoundness warnings for GTK's `glib 0.18.5` variant-string iterator and the old `rand 0.7.3` implementation pulled through Tauri's parsing dependencies. mqx does not call the GLib iterator API or configure a custom logger that recursively calls rand. These are residual upstream risks; they are not silently suppressed or described as fixed. Revisit them when Tauri's GTK/dependency stack updates.

The automated audit checks published dependency advisories. It is not a full security assessment of mqx or proof that dependencies have no vulnerabilities.
