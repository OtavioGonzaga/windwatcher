# ![windwatcher](docs/assets/windwatcher-banner.png)

An embeddable, self-hostable messaging engine written in Rust.

## Status

Windwatcher is in early development. The repository currently provides project
foundations only and is not production-ready.

## What is Windwatcher?

Windwatcher is intended to provide messaging capabilities that Rust applications
can embed directly or run through an official self-hosted server.

## Workspace

- [`windwatcher`](crates/windwatcher): embeddable Rust library.
- [`windwatcher-server`](crates/windwatcher-server): official standalone,
  self-hosted server built on the library. Its executable is named `windwatcher`.

The packages are independently versioned.

## Operator commands

```bash
windwatcher migrate
windwatcher user create --username otavio
windwatcher config check
```

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
