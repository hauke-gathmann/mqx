use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
    time::Duration,
};

use anyhow::{Context, Result};
use rumqttc::{Event, Incoming, QoS};

use crate::{
    broker::{self, Broker},
    event::{TraceEvent, now_ms},
};

pub struct RecordOpts {
    pub broker: Broker,
    pub output: PathBuf,
    pub topics: Vec<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub client_id: String,
}

pub async fn run(opts: RecordOpts) -> Result<()> {
    if let Some(parent) = opts.output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }

    let file =
        File::create(&opts.output).with_context(|| format!("create {}", opts.output.display()))?;
    let mut out = BufWriter::new(file);

    let (client, mut eventloop) = broker::connect(
        &opts.broker,
        opts.client_id.clone(),
        opts.username,
        opts.password,
    );

    eprintln!(
        "recording {} → {} (Ctrl+C to stop)",
        opts.broker.display(),
        opts.output.display()
    );

    let mut subscribed = false;
    let mut count: u64 = 0;
    let started = now_ms();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break;
            }
            event = eventloop.poll() => {
                match event {
                    Ok(Event::Incoming(Incoming::ConnAck(_))) => {
                        if !subscribed {
                            for topic in &opts.topics {
                                client
                                    .subscribe(topic, QoS::AtLeastOnce)
                                    .await
                                    .with_context(|| format!("subscribe {topic}"))?;
                            }
                            subscribed = true;
                            eprintln!("subscribed to {}", opts.topics.join(", "));
                        }
                    }
                    Ok(Event::Incoming(Incoming::Publish(publish))) => {
                        let event = TraceEvent::from_publish(&publish, now_ms());
                        writeln!(out, "{}", event.to_jsonl()?)
                            .context("write trace event")?;
                        out.flush().context("flush trace")?;
                        count += 1;
                        if count == 1 || count.is_multiple_of(100) {
                            eprintln!("captured {count} messages");
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("connection error: {error}; retrying");
                        subscribed = false;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
            }
        }
    }

    out.flush().ok();
    let elapsed_ms = now_ms().saturating_sub(started);
    eprintln!(
        "stopped after {:.1}s, wrote {count} messages to {}",
        elapsed_ms as f64 / 1000.0,
        opts.output.display()
    );
    Ok(())
}
