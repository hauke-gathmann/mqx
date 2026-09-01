use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use rumqttc::{Event, Incoming};
use tokio::time::{Instant, sleep};

use crate::{
    broker::{self, Broker, qos_from_u8},
    event::{self, TraceEvent},
};

pub struct ReplayOpts {
    pub broker: Broker,
    pub input: PathBuf,
    pub username: Option<String>,
    pub password: Option<String>,
    pub client_id: String,
    pub speed: f64,
    pub loop_trace: bool,
}

pub fn wait_duration(t0_ms: u64, t_ms: u64, speed: f64, elapsed: Duration) -> Duration {
    if !speed.is_finite() || speed <= 0.0 {
        return Duration::ZERO;
    }
    let recorded = Duration::from_millis(t_ms.saturating_sub(t0_ms));
    let scaled = recorded.div_f64(speed);
    scaled.saturating_sub(elapsed)
}

pub async fn run(opts: ReplayOpts) -> Result<()> {
    if opts.speed < 0.0 || !opts.speed.is_finite() {
        bail!("--speed must be a finite number >= 0 (0 = as fast as possible)");
    }

    let events = load_events(&opts.input)?;
    if events.is_empty() {
        bail!("{} contains no events", opts.input.display());
    }

    let span_ms = events.last().unwrap().t_ms.saturating_sub(events[0].t_ms);
    eprintln!(
        "replaying {} events from {} onto {} ({:.1}s recorded, speed {})",
        events.len(),
        opts.input.display(),
        opts.broker.display(),
        span_ms as f64 / 1000.0,
        if opts.speed == 0.0 {
            "max".to_string()
        } else {
            format!("{}x", opts.speed)
        }
    );

    let (client, mut eventloop) = broker::connect(
        &opts.broker,
        opts.client_id.clone(),
        opts.username,
        opts.password,
    );

    let mut connected = false;
    while !connected {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("cancelled before connect");
                return Ok(());
            }
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => connected = true,
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("connection error: {error}; retrying");
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }
    }

    let pump = tokio::spawn(async move {
        loop {
            if eventloop.poll().await.is_err() {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
    });

    let mut round: u64 = 0;
    loop {
        round += 1;
        let t0 = events[0].t_ms;
        let start = Instant::now();
        for (i, event) in events.iter().enumerate() {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    pump.abort();
                    eprintln!("stopped after {} publishes", i + round.saturating_sub(1) as usize * events.len());
                    return Ok(());
                }
                _ = sleep(wait_duration(t0, event.t_ms, opts.speed, start.elapsed())) => {}
            }

            let payload = event.payload_bytes()?;
            client
                .publish(&event.topic, qos_from_u8(event.qos), event.retain, payload)
                .await
                .with_context(|| format!("publish {}", event.topic))?;
        }

        eprintln!(
            "finished round {round} ({} messages in {:.1}s)",
            events.len(),
            start.elapsed().as_secs_f64()
        );
        if !opts.loop_trace {
            break;
        }
    }

    pump.abort();
    Ok(())
}

fn load_events(path: &PathBuf) -> Result<Vec<TraceEvent>> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    load_events_from(BufReader::new(file), path)
}

fn load_events_from<R: BufRead>(reader: R, path: &Path) -> Result<Vec<TraceEvent>> {
    let mut events = Vec::new();
    let mut first_nonempty = true;
    for (i, line) in reader.lines().enumerate() {
        let line = line.with_context(|| format!("read {} line {}", path.display(), i + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        if first_nonempty {
            first_nonempty = false;
            if event::is_recording_header(&line) {
                continue;
            }
        }
        events.push(
            TraceEvent::from_jsonl(&line)
                .with_context(|| format!("{}:{}", path.display(), i + 1))?,
        );
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realtime_waits_recorded_gap() {
        let wait = wait_duration(1_000, 1_250, 1.0, Duration::from_millis(50));
        assert_eq!(wait, Duration::from_millis(200));
    }

    #[test]
    fn double_speed_halves_wait() {
        let wait = wait_duration(0, 1_000, 2.0, Duration::ZERO);
        assert_eq!(wait, Duration::from_millis(500));
    }

    #[test]
    fn max_speed_does_not_wait() {
        assert_eq!(wait_duration(0, 5_000, 0.0, Duration::ZERO), Duration::ZERO);
    }

    #[test]
    fn late_clock_does_not_sleep() {
        assert_eq!(
            wait_duration(0, 100, 1.0, Duration::from_millis(150)),
            Duration::ZERO
        );
    }

    #[test]
    fn load_events_skips_in_app_header() {
        let data = concat!(
            r#"{"kind":"mqx-recording","v":1,"startedAt":"2026-08-26T12:00:00.000Z","endedAt":"2026-08-26T12:00:32.100Z","profileId":"p","profileName":"n","broker":"mqtt://localhost:1883","messages":1,"topics":1,"appVersion":"0.2.0"}"#,
            "\n",
            r#"{"t_ms":1787735165782,"topic":"home/lamp","qos":0,"retain":false,"dup":false,"payload":"AAEC/w=="}"#,
            "\n",
        );
        let events = load_events_from(data.as_bytes(), &PathBuf::from("header.jsonl")).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].topic, "home/lamp");
        assert_eq!(events[0].payload_bytes().unwrap(), vec![0, 1, 2, 255]);
    }

    #[test]
    fn load_events_headerless_mqtt_trace() {
        let data = concat!(
            r#"{"t_ms":1,"topic":"a","qos":0,"retain":false,"payload":"YQ=="}"#,
            "\n",
            r#"{"t_ms":2,"topic":"b","qos":1,"retain":true,"dup":true,"payload":"Yg=="}"#,
            "\n",
        );
        let events = load_events_from(data.as_bytes(), &PathBuf::from("trace.jsonl")).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].topic, "a");
        assert!(!events[0].dup);
        assert_eq!(events[1].topic, "b");
        assert!(events[1].dup);
    }
}
