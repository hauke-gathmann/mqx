use std::{
    fs,
    io::{BufReader, Cursor},
    sync::Arc,
};

use rumqttc::{MqttOptions, Transport};
use rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, RootCertStore, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
};

use crate::{
    error::{Error, Result},
    profiles::{ConnectionProfile, Protocol, TlsConfig},
};

pub fn broker_display(profile: &ConnectionProfile) -> String {
    format!(
        "{}://{}:{}",
        profile.protocol.as_str(),
        profile.host,
        profile.port
    )
}

pub fn mqtt_options(profile: &ConnectionProfile, password: Option<String>) -> Result<MqttOptions> {
    let client_id = if profile.client_id.is_empty() {
        let suffix = profile.id.get(..8).unwrap_or(profile.id.as_str());
        format!("mqx-{suffix}")
    } else {
        profile.client_id.clone()
    };

    let host = match profile.protocol {
        // rumqttc reads the WS request URL from broker_addr, not host+port.
        Protocol::Ws | Protocol::Wss => websocket_url(profile),
        Protocol::Mqtt | Protocol::Mqtts => profile.host.clone(),
    };

    let mut options = MqttOptions::new(client_id, host, profile.port);
    options
        .set_clean_session(profile.session.clean)
        .set_keep_alive(std::time::Duration::from_secs(
            profile.session.keep_alive_secs.max(1),
        ));
    let max = profile.session.max_packet_size as usize;
    options.set_max_packet_size(max, max);

    if !profile.username.is_empty() {
        options.set_credentials(&profile.username, password.unwrap_or_default());
    }

    if let Some(will) = &profile.last_will {
        options.set_last_will(rumqttc::LastWill::new(
            &will.topic,
            will.payload.as_bytes().to_vec(),
            qos_from_u8(will.qos),
            will.retain,
        ));
    }

    options.set_transport(transport(profile)?);
    Ok(options)
}

fn websocket_url(profile: &ConnectionProfile) -> String {
    let scheme = match profile.protocol {
        Protocol::Wss => "wss",
        _ => "ws",
    };
    let path = profile
        .websocket_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or("/mqtt");
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{scheme}://{}:{}{path}", profile.host, profile.port)
}

pub fn qos_from_u8(value: u8) -> rumqttc::QoS {
    match value {
        1 => rumqttc::QoS::AtLeastOnce,
        2 => rumqttc::QoS::ExactlyOnce,
        _ => rumqttc::QoS::AtMostOnce,
    }
}

pub fn qos_from_rumqttc(qos: rumqttc::QoS) -> crate::message::QoS {
    match qos {
        rumqttc::QoS::AtMostOnce => crate::message::QoS::AtMostOnce,
        rumqttc::QoS::AtLeastOnce => crate::message::QoS::AtLeastOnce,
        rumqttc::QoS::ExactlyOnce => crate::message::QoS::ExactlyOnce,
    }
}

fn transport(profile: &ConnectionProfile) -> Result<Transport> {
    match profile.protocol {
        Protocol::Mqtt => Ok(Transport::Tcp),
        Protocol::Mqtts => Ok(Transport::Tls(tls_config(&profile.tls)?)),
        Protocol::Ws => Ok(Transport::Ws),
        Protocol::Wss => Ok(Transport::Wss(tls_config(&profile.tls)?)),
    }
}

fn tls_config(tls: &TlsConfig) -> Result<rumqttc::TlsConfiguration> {
    if !tls.validate {
        return Ok(rumqttc::TlsConfiguration::Rustls(Arc::new(
            insecure_client_config(tls)?,
        )));
    }

    let ca = read_optional(tls.ca_cert_path.as_ref())?.unwrap_or_default();
    let client_auth = match (
        read_optional(tls.client_cert_path.as_ref())?,
        read_optional(tls.client_key_path.as_ref())?,
    ) {
        (Some(cert), Some(key)) if !cert.is_empty() && !key.is_empty() => Some((cert, key)),
        _ => None,
    };
    let alpn = tls
        .alpn
        .as_ref()
        .map(|items| items.iter().map(|s| s.as_bytes().to_vec()).collect());

    if ca.is_empty() {
        // rumqttc Simple rejects an empty CA; webpki roots match "validate against public CAs".
        return Ok(rumqttc::TlsConfiguration::Rustls(Arc::new(
            webpki_client_config(tls, client_auth, alpn)?,
        )));
    }

    Ok(rumqttc::TlsConfiguration::Simple {
        ca,
        alpn,
        client_auth,
    })
}

fn webpki_client_config(
    tls: &TlsConfig,
    client_auth: Option<(Vec<u8>, Vec<u8>)>,
    alpn: Option<Vec<Vec<u8>>>,
) -> Result<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    finish_client_config(roots, tls, client_auth, alpn)
}

fn insecure_client_config(tls: &TlsConfig) -> Result<ClientConfig> {
    let builder = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert));
    let mut config = apply_client_auth(builder, tls)?;
    if let Some(alpn) = &tls.alpn {
        config.alpn_protocols = alpn.iter().map(|s| s.as_bytes().to_vec()).collect();
    }
    Ok(config)
}

fn finish_client_config(
    roots: RootCertStore,
    _tls: &TlsConfig,
    client_auth: Option<(Vec<u8>, Vec<u8>)>,
    alpn: Option<Vec<Vec<u8>>>,
) -> Result<ClientConfig> {
    let builder = ClientConfig::builder().with_root_certificates(roots);
    let mut config = match client_auth {
        Some((cert, key)) => {
            let certs = load_certs(&cert)?;
            let key = load_key(&key)?;
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| Error::Tls(e.to_string()))?
        }
        None => builder.with_no_client_auth(),
    };
    if let Some(alpn) = alpn {
        config.alpn_protocols = alpn;
    }
    Ok(config)
}

fn apply_client_auth(
    builder: rustls::ConfigBuilder<ClientConfig, rustls::client::WantsClientCert>,
    tls: &TlsConfig,
) -> Result<ClientConfig> {
    match (
        read_optional(tls.client_cert_path.as_ref())?,
        read_optional(tls.client_key_path.as_ref())?,
    ) {
        (Some(cert), Some(key)) if !cert.is_empty() && !key.is_empty() => {
            let certs = load_certs(&cert)?;
            let key = load_key(&key)?;
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| Error::Tls(e.to_string()))
        }
        _ => Ok(builder.with_no_client_auth()),
    }
}

fn read_optional(path: Option<&std::path::PathBuf>) -> Result<Option<Vec<u8>>> {
    let Some(path) = path else {
        return Ok(None);
    };
    fs::read(path).map(Some).map_err(|source| Error::CertFile {
        path: path.clone(),
        source,
    })
}

fn load_certs(bytes: &[u8]) -> Result<Vec<CertificateDer<'static>>> {
    let pem = rustls_pemfile::certs(&mut BufReader::new(Cursor::new(bytes)))
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap_or_default();
    if !pem.is_empty() {
        return Ok(pem);
    }
    if looks_like_pem(bytes) {
        return Err(Error::Tls("no certificates in PEM".into()));
    }
    if bytes.is_empty() {
        return Err(Error::Tls("empty certificate".into()));
    }
    Ok(vec![CertificateDer::from(bytes.to_vec())])
}

fn load_key(bytes: &[u8]) -> Result<PrivateKeyDer<'static>> {
    let mut reader = BufReader::new(Cursor::new(bytes));
    loop {
        match rustls_pemfile::read_one(&mut reader) {
            Ok(Some(rustls_pemfile::Item::Pkcs8Key(key))) => return Ok(PrivateKeyDer::Pkcs8(key)),
            Ok(Some(rustls_pemfile::Item::Pkcs1Key(key))) => return Ok(PrivateKeyDer::Pkcs1(key)),
            Ok(Some(rustls_pemfile::Item::Sec1Key(key))) => return Ok(PrivateKeyDer::Sec1(key)),
            Ok(None) => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    if looks_like_pem(bytes) || bytes.is_empty() {
        return Err(Error::Tls("no private key in PEM".into()));
    }
    Ok(PrivateKeyDer::Pkcs8(bytes.to_vec().into()))
}

fn looks_like_pem(bytes: &[u8]) -> bool {
    bytes.windows(11).any(|w| w == b"-----BEGIN ")
}

/// WHY: homelab brokers often use self-signed certs (MQTT Explorer
/// "reject unauthorized" off). Only used when the profile sets `validate: false`.
#[derive(Debug)]
struct AcceptAnyServerCert;

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::{LastWill, SessionConfig};

    fn sample() -> ConnectionProfile {
        ConnectionProfile {
            id: "abcd1234ef".into(),
            name: "Home".into(),
            protocol: Protocol::Mqtt,
            host: "localhost".into(),
            port: 1883,
            client_id: "mqx-test".into(),
            username: "hauke".into(),
            tls: TlsConfig::default(),
            session: SessionConfig::default(),
            subscriptions: Vec::new(),
            last_will: None,
            websocket_path: None,
        }
    }

    #[test]
    fn broker_display_strips_nothing_password_never_present() {
        let mut profile = sample();
        profile.protocol = Protocol::Mqtts;
        profile.host = "ha.local".into();
        profile.port = 8883;
        assert_eq!(broker_display(&profile), "mqtts://ha.local:8883");
    }

    #[test]
    fn mqtt_options_plain_tcp() {
        let profile = sample();
        let opts = mqtt_options(&profile, Some("secret".into())).unwrap();
        assert_eq!(opts.broker_address(), ("localhost".into(), 1883));
        assert!(!opts.clean_session());
        assert_eq!(opts.keep_alive(), std::time::Duration::from_secs(60));
        assert_eq!(opts.credentials(), Some(("hauke".into(), "secret".into())));
        assert!(matches!(opts.transport(), Transport::Tcp));
    }

    #[test]
    fn mqtt_options_ws_uses_url_host() {
        let mut profile = sample();
        profile.protocol = Protocol::Ws;
        profile.port = 8083;
        let opts = mqtt_options(&profile, None).unwrap();
        let (host, port) = opts.broker_address();
        assert_eq!(host, "ws://localhost:8083/mqtt");
        assert_eq!(port, 8083);
        assert!(matches!(opts.transport(), Transport::Ws));
        assert_eq!(broker_display(&profile), "ws://localhost:8083");
    }

    #[test]
    fn mqtt_options_ws_custom_path() {
        let mut profile = sample();
        profile.protocol = Protocol::Ws;
        profile.port = 8083;
        profile.websocket_path = Some("broker/ws".into());
        let opts = mqtt_options(&profile, None).unwrap();
        let (host, _) = opts.broker_address();
        assert_eq!(host, "ws://localhost:8083/broker/ws");
    }

    #[test]
    fn mqtt_options_wss_insecure() {
        let mut profile = sample();
        profile.protocol = Protocol::Wss;
        profile.port = 8084;
        profile.tls.validate = false;
        let opts = mqtt_options(&profile, None).unwrap();
        let (host, _) = opts.broker_address();
        assert_eq!(host, "wss://localhost:8084/mqtt");
        assert!(matches!(opts.transport(), Transport::Wss(_)));
    }

    #[test]
    fn mqtt_options_wss_webpki_when_no_ca() {
        let mut profile = sample();
        profile.protocol = Protocol::Wss;
        profile.port = 8084;
        profile.tls.validate = true;
        profile.tls.ca_cert_path = None;
        let opts = mqtt_options(&profile, None).unwrap();
        assert!(matches!(opts.transport(), Transport::Wss(_)));
    }

    #[test]
    fn mqtt_options_mqtts_simple_when_ca_present() {
        let dir = tempfile::tempdir().unwrap();
        let ca = dir.path().join("ca.pem");
        std::fs::write(
            &ca,
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        let mut profile = sample();
        profile.protocol = Protocol::Mqtts;
        profile.port = 8883;
        profile.tls.validate = true;
        profile.tls.ca_cert_path = Some(ca);
        let opts = mqtt_options(&profile, None).unwrap();
        assert!(matches!(opts.transport(), Transport::Tls(_)));
    }

    #[test]
    fn load_certs_accepts_raw_der() {
        let der = vec![0x30, 0x03, 0x02, 0x01, 0x00];
        let certs = load_certs(&der).unwrap();
        assert_eq!(certs.len(), 1);
        assert_eq!(certs[0].as_ref(), der);
    }

    #[test]
    fn mqtt_options_last_will_and_clean() {
        let mut profile = sample();
        profile.session.clean = true;
        profile.last_will = Some(LastWill {
            topic: "clients/ui".into(),
            payload: "gone".into(),
            qos: 1,
            retain: false,
        });
        let opts = mqtt_options(&profile, None).unwrap();
        assert!(opts.clean_session());
        assert!(opts.last_will().is_some());
    }

    #[test]
    fn missing_ca_file_is_error() {
        let mut profile = sample();
        profile.protocol = Protocol::Mqtts;
        profile.tls.ca_cert_path = Some("/no/such/ca.pem".into());
        let err = mqtt_options(&profile, None).unwrap_err();
        assert!(matches!(err, Error::CertFile { .. }));
    }
}
