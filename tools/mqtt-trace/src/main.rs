mod broker;
mod event;
mod record;
mod replay;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::broker::Broker;

#[derive(Parser)]
#[command(
    name = "mqtt-trace",
    about = "Record all MQTT traffic on a broker, then replay it in real time.",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Subscribe to the broker and write every publish to a JSONL trace.
    Record(RecordArgs),
    /// Publish a recorded trace back to a broker, preserving inter-arrival times.
    Replay(ReplayArgs),
}

#[derive(clap::Args)]
struct RecordArgs {
    /// Broker URL: mqtt://host:port, mqtts://host:port, or host:port
    #[arg(short, long, default_value = "mqtt://127.0.0.1:1883")]
    broker: String,
    /// Trace file to write (JSONL)
    #[arg(short, long, default_value = "trace.jsonl")]
    output: PathBuf,
    /// Topic filter to subscribe (repeatable). Default: #
    #[arg(short = 't', long = "topic")]
    topics: Vec<String>,
    /// Also subscribe to $SYS/# (not covered by #)
    #[arg(long)]
    sys: bool,
    #[arg(short, long)]
    username: Option<String>,
    #[arg(short, long, env = "MQTT_TRACE_PASSWORD")]
    password: Option<String>,
    #[arg(long)]
    client_id: Option<String>,
}

#[derive(clap::Args)]
struct ReplayArgs {
    /// Trace file to read (JSONL from `mqtt-trace record`)
    file: PathBuf,
    /// Broker URL: mqtt://host:port, mqtts://host:port, or host:port
    #[arg(short, long, default_value = "mqtt://127.0.0.1:1883")]
    broker: String,
    /// Playback speed. 1 = real time, 2 = twice as fast, 0 = as fast as possible
    #[arg(long, default_value_t = 1.0)]
    speed: f64,
    /// Repeat the trace until interrupted
    #[arg(long = "loop")]
    loop_trace: bool,
    #[arg(short, long)]
    username: Option<String>,
    #[arg(short, long, env = "MQTT_TRACE_PASSWORD")]
    password: Option<String>,
    #[arg(long)]
    client_id: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Record(args) => {
            let mut topics = args.topics;
            if topics.is_empty() {
                topics.push("#".into());
            }
            if args.sys && !topics.iter().any(|t| t == "$SYS/#") {
                topics.push("$SYS/#".into());
            }
            record::run(record::RecordOpts {
                broker: Broker::parse(&args.broker)?,
                output: args.output,
                topics,
                username: args.username,
                password: args.password,
                client_id: args.client_id.unwrap_or_else(|| unique_id("record")),
            })
            .await
        }
        Command::Replay(args) => {
            replay::run(replay::ReplayOpts {
                broker: Broker::parse(&args.broker)?,
                input: args.file,
                username: args.username,
                password: args.password,
                client_id: args.client_id.unwrap_or_else(|| unique_id("replay")),
                speed: args.speed,
                loop_trace: args.loop_trace,
            })
            .await
        }
    }
}

fn unique_id(kind: &str) -> String {
    format!(
        "mqx-trace-{kind}-{}-{}",
        std::process::id(),
        now_bits() % 10_000
    )
}

fn now_bits() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
}
