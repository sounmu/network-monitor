//! Port-status probing and the comma-separated parsing helper used
//! by the `/metrics` query string.

use crate::models::PortStatus;
use std::time::Duration;

/// Parse a comma-separated port string into `Vec<u16>`.
/// Invalid values (out-of-range, non-numeric) are silently ignored.
pub(crate) fn parse_comma_separated_ports(input: &str) -> Vec<u16> {
    input
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

#[tracing::instrument]
pub(crate) async fn collect_ports(ports: Vec<u16>) -> Vec<PortStatus> {
    let futs = ports.into_iter().map(|port| async move {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let is_open = tokio::time::timeout(
            Duration::from_millis(100),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        .is_ok_and(|r| r.is_ok());
        PortStatus { port, is_open }
    });
    futures_util::future::join_all(futs).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_parsing_filters_invalid_values() {
        assert_eq!(
            parse_comma_separated_ports("80,443,invalid,8080"),
            vec![80, 443, 8080],
            "Invalid ports should be silently removed"
        );
    }

    #[test]
    fn test_port_parsing_trims_whitespace() {
        assert_eq!(
            parse_comma_separated_ports(" 80 , 443 , 3000 "),
            vec![80, 443, 3000]
        );
    }

    #[test]
    fn test_port_parsing_empty_string_returns_empty() {
        assert!(parse_comma_separated_ports("").is_empty());
    }

    #[test]
    fn test_port_parsing_rejects_out_of_range() {
        // Values exceeding u16::MAX (65535) fail to parse and are dropped.
        assert_eq!(parse_comma_separated_ports("80,65536,443"), vec![80, 443]);
    }
}
