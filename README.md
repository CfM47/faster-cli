# faster-cli

Measure the speed of your internet connection from the terminal.

```
$ fast
Latency    204.6 ms (±20.5 ms jitter)
Download   3.8 Mbps
Upload     9.9 Mbps
```

## Install

With a Rust toolchain:

```bash
cargo install faster-cli
```

## Usage

```bash
fast                  # latency, download and upload
fast --download-only  # skip the upload phase
fast --json           # one JSON object, for scripts
fast --verbose        # also show which server answered
```

Attached to a terminal, the current rate is drawn full screen while the test
runs. Piped or redirected, nothing is printed until the result is ready, so
`fast --json > result.json` gives you exactly the object and nothing else.

Exit codes are `0` on success, `1` on failure, `2` for a bad argument and
`130` if you interrupt it with ctrl-c.

### JSON output

`--json` always includes every field, whether or not `--verbose` is given. A
skipped upload is `null` rather than an absent key, so `jq` expressions keep
working across both modes.

```json
{
  "server": { "host": "speed.cloudflare.com", "colo": "MAD" },
  "client": { "ip": "203.0.113.7", "country": "ES" },
  "latency_ms": 13.4,
  "jitter_ms": 1.2,
  "download_mbps": 243.8,
  "downloaded_bytes": 304750000,
  "upload_mbps": 41.2,
  "uploaded_bytes": 51500000,
  "duration_s": 21.3
}
```

Rates are rounded to two decimals. The byte counts and duration are exact, so
you can recompute a rate at full precision if you need to.

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- --verbose
```

## License

MIT
