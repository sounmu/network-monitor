//! Port-status probing and the comma-separated parsing helper used
//! by the `/metrics` query string.

use crate::models::PortStatus;
use std::time::Duration;

/// Parse a comma-separated port string into `Vec<u16>`, capped at `max`
/// entries. Invalid values (out-of-range, non-numeric) are silently ignored.
///
/// The cap is applied via `.take(max)` **during** iteration rather than
/// `.truncate()` after collect, so a hostile query string with tens of
/// thousands of entries cannot force us to materialise the full Vec before
/// trimming (CLAUDE.md §Security "Port scan cap").
pub(crate) fn parse_comma_separated_ports(input: &str, max: usize) -> Vec<u16> {
    input
        .split(',')
        .filter_map(|s| s.trim().parse::<u16>().ok())
        .filter(|&p| p > 0)
        .take(max)
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
            parse_comma_separated_ports("80,443,invalid,8080", 100),
            vec![80, 443, 8080],
            "Invalid ports should be silently removed"
        );
    }

    #[test]
    fn test_port_parsing_trims_whitespace() {
        assert_eq!(
            parse_comma_separated_ports(" 80 , 443 , 3000 ", 100),
            vec![80, 443, 3000]
        );
    }

    #[test]
    fn test_port_parsing_empty_string_returns_empty() {
        assert!(parse_comma_separated_ports("", 100).is_empty());
    }

    #[test]
    fn test_port_parsing_rejects_out_of_range() {
        // Values exceeding u16::MAX (65535) fail to parse and are dropped.
        assert_eq!(
            parse_comma_separated_ports("80,65536,443", 100),
            vec![80, 443]
        );
    }

    #[test]
    fn test_port_parsing_caps_iteration_before_collect() {
        // Pathological input: 50_000 valid ports. Pre-fix behaviour was to
        // allocate the full Vec and truncate after, giving an attacker a DoS
        // amplifier; verify the take(max) cap short-circuits iteration.
        let mut big = String::with_capacity(50_000 * 6);
        for i in 1..=50_000u32 {
            if i > 1 {
                big.push(',');
            }
            big.push_str(&i.to_string());
        }
        let out = parse_comma_separated_ports(&big, 100);
        assert_eq!(out.len(), 100);
        assert_eq!(out[0], 1);
        assert_eq!(out[99], 100);
    }
}
