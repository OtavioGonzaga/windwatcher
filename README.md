# ![windwatcher](docs/assets/windwatcher-banner.png)

An embeddable, self-hostable messaging engine written in Rust.

## Status

Windwatcher is in early development. The repository currently provides project
an embeddable engine and standalone HTTP/realtime service, but is not production-ready.

## What is Windwatcher?

Windwatcher is intended to provide messaging capabilities that Rust applications
can embed directly or run through an official self-hosted server.

## Workspace

- [`windwatcher`](crates/windwatcher): embeddable Rust library.
- [`windwatcher-server`](crates/windwatcher-server): official standalone,
  self-hosted server built on the library. Its executable is named `windwatcher`.

The packages are independently versioned.

## Operator commands

Configure a stable JWT signing secret first; see [server configuration](docs/server-configuration.md).

```bash
windwatcher migrate
windwatcher user create --username alice
windwatcher doctor
windwatcher serve
```

Plain `windwatcher` is equivalent to `windwatcher serve`, listening on
`127.0.0.1:8080` by default. See [operations](docs/operations.md) for health,
diagnostics and graceful shutdown. The optional durable queue and separate
`windwatcher worker` process are documented in
[Durable jobs](docs/jobs.md); no production job handlers are shipped yet.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for development requirements, workflow,
and local validation.

## Documentation

See the [documentation index](docs/README.md).

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) before
opening a change.

## License

Windwatcher is licensed under the [MIT License](LICENSE).
