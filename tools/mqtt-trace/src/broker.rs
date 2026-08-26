use anyhow::{Context, Result, bail};
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS, Transport};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broker {
    pub host: String,
    pub port: u16,
    pub tls: bool,
}

impl Broker {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            bail!("broker URL is empty");
        }

        let (tls, rest, default_port) = if let Some(rest) = input.strip_prefix("mqtts://") {
            (true, rest, 8883_u16)
        } else if let Some(rest) = input.strip_prefix("mqtt://") {
            (false, rest, 1883_u16)
        } else if input.contains("://") {
            bail!("unsupported broker scheme in {input:?}; use mqtt:// or mqtts://");
        } else {
            (false, input, 1883_u16)
        };

        let rest = rest.split('/').next().unwrap_or(rest);
        let rest = rest.rsplit('@').next().unwrap_or(rest);
        if rest.is_empty() {
            bail!("broker host is empty");
        }

        let (host, port) = split_host_port(rest, default_port)?;
        if host.is_empty() {
            bail!("broker host is empty");
        }
        Ok(Self { host, port, tls })
    }

    pub fn display(&self) -> String {
        let scheme = if self.tls { "mqtts" } else { "mqtt" };
        if self.host.contains(':') {
            format!("{scheme}://[{}]:{}", self.host, self.port)
        } else {
            format!("{scheme}://{}:{}", self.host, self.port)
        }
    }
}

fn split_host_port(rest: &str, default_port: u16) -> Result<(String, u16)> {
    if let Some(rest) = rest.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .context("invalid IPv6 broker address (missing ']')")?;
        let port = if let Some(port) = tail.strip_prefix(':') {
            parse_port(port)?
        } else if tail.is_empty() {
            default_port
        } else {
            bail!("invalid IPv6 broker address");
        };
        return Ok((host.to_string(), port));
    }

    if let Some((host, port)) = rest.rsplit_once(':')
        && !host.is_empty()
        && !host.contains(':')
        && !port.is_empty()
        && port.chars().all(|c| c.is_ascii_digit())
    {
        return Ok((host.to_string(), parse_port(port)?));
    }

    Ok((rest.to_string(), default_port))
}

fn parse_port(port: &str) -> Result<u16> {
    port.parse()
        .with_context(|| format!("invalid broker port {port:?}"))
}

pub fn connect(
    broker: &Broker,
    client_id: impl Into<String>,
    username: Option<String>,
    password: Option<String>,
) -> (AsyncClient, EventLoop) {
    let mut options = MqttOptions::new(client_id, broker.host.clone(), broker.port);
    options.set_keep_alive(Duration::from_secs(30));
    options.set_clean_session(true);
    options.set_max_packet_size(8 * 1024 * 1024, 8 * 1024 * 1024);
    if let Some(user) = username {
        options.set_credentials(user, password.unwrap_or_default());
    }
    if broker.tls {
        options.set_transport(Transport::tls_with_default_config());
    }
    AsyncClient::new(options, 64)
}

pub fn qos_from_u8(value: u8) -> QoS {
    match value {
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtMostOnce,
    }
}

pub fn qos_to_u8(qos: QoS) -> u8 {
    match qos {
        QoS::AtMostOnce => 0,
        QoS::AtLeastOnce => 1,
        QoS::ExactlyOnce => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_host() {
        assert_eq!(
            Broker::parse("localhost").unwrap(),
            Broker {
                host: "localhost".into(),
                port: 1883,
                tls: false,
            }
        );
    }

    #[test]
    fn parses_host_port() {
        assert_eq!(
            Broker::parse("127.0.0.1:1884").unwrap(),
            Broker {
                host: "127.0.0.1".into(),
                port: 1884,
                tls: false,
            }
        );
    }

    #[test]
    fn parses_mqtt_url() {
        assert_eq!(
            Broker::parse("mqtt://broker.local").unwrap(),
            Broker {
                host: "broker.local".into(),
                port: 1883,
                tls: false,
            }
        );
    }

    #[test]
    fn parses_mqtts_url_with_port() {
        assert_eq!(
            Broker::parse("mqtts://broker.local:8884/ignored").unwrap(),
            Broker {
                host: "broker.local".into(),
                port: 8884,
                tls: true,
            }
        );
    }

    #[test]
    fn parses_ipv6() {
        assert_eq!(
            Broker::parse("mqtt://[::1]:1883").unwrap(),
            Broker {
                host: "::1".into(),
                port: 1883,
                tls: false,
            }
        );
    }

    #[test]
    fn rejects_unknown_scheme() {
        let err = Broker::parse("ws://localhost:9001").unwrap_err();
        assert!(err.to_string().contains("unsupported"));
    }
}
