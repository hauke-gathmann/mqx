# tools

## mqtt-trace

Record all publishes on an MQTT broker, then replay them in real time.

```bash
# Capture (# by default). Ctrl+C stops.
cargo run -p mqtt-trace -- record --broker mqtt://127.0.0.1:1883 -o traffic.jsonl

# Play back with original timing
cargo run -p mqtt-trace -- replay traffic.jsonl --broker mqtt://127.0.0.1:1883
```

Useful flags:

- `--topic home/#` (repeatable; default `#`)
- `--sys` also subscribe `$SYS/#`
- `--speed 2` twice as fast; `--speed 0` as fast as possible
- `--loop` repeat the trace until Ctrl+C
- `--username` / `--password` (or `MQTT_TRACE_PASSWORD`)
