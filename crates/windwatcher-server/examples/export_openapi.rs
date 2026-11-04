//! Export the router's OpenAPI contract to stdout without starting the server.

use std::io::{self, Write};

fn export(writer: impl Write) -> Result<(), serde_json::Error> {
    serde_json::to_writer_pretty(writer, &windwatcher_server::http::openapi())
}

fn main() -> Result<(), serde_json::Error> {
    export(io::stdout().lock())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_the_router_contract_without_configuration_or_services() {
        let mut output = Vec::new();
        export(&mut output).unwrap();
        let actual: serde_json::Value = serde_json::from_slice(&output).unwrap();
        let expected = serde_json::to_value(windwatcher_server::http::openapi()).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual["openapi"], "3.1.0");
        assert_eq!(actual["info"]["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn propagates_output_failures() {
        struct BrokenOutput;
        impl Write for BrokenOutput {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed output"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(export(BrokenOutput).unwrap_err().is_io());
    }
}
