use crate::ResolvedHttpConfig;
use crate::http::{
    middleware::request_id::RequestContext,
    problem::{ApiProblem, ProblemKind},
};
use axum::{
    extract::{ConnectInfo, MatchedPath, Request, State},
    http::{Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use governor::{
    DefaultKeyedRateLimiter, RateLimiter,
    clock::{Clock, DefaultClock},
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone)]
pub(in crate::http) struct Limits(Arc<Inner>);
struct Inner {
    trusted_proxies: Vec<ipnet::IpNet>,
    general: DefaultKeyedRateLimiter<IpAddr>,
    auth: DefaultKeyedRateLimiter<IpAddr>,
    requests: AtomicU64,
}
impl Limits {
    pub fn new(config: &ResolvedHttpConfig) -> Self {
        Self(Arc::new(Inner {
            trusted_proxies: config.trusted_proxies.clone(),
            general: RateLimiter::keyed(config.general),
            auth: RateLimiter::keyed(config.auth),
            requests: AtomicU64::new(0),
        }))
    }
}

pub async fn enforce(State(limits): State<Limits>, request: Request, next: Next) -> Response {
    let context = *request
        .extensions()
        .get::<RequestContext>()
        .expect("outer context");
    let Some(ConnectInfo(peer)) = request.extensions().get::<ConnectInfo<SocketAddr>>() else {
        return ApiProblem::internal(
            context,
            &std::io::Error::other("HTTP peer ConnectInfo missing"),
        )
        .into_response();
    };
    let ip = client_ip(peer.ip(), request.headers(), &limits.0.trusted_proxies);
    // Opportunistic governor cleanup: no background runtime/lifecycle is needed.
    if limits
        .0
        .requests
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(1024)
    {
        limits.0.general.retain_recent();
        limits.0.auth.retain_recent();
    }
    let auth_request = request.method() == Method::POST
        && request
            .extensions()
            .get::<MatchedPath>()
            .is_some_and(|path| {
                matches!(
                    path.as_str(),
                    "/v1/users" | "/v1/sessions" | "/v1/sessions/refresh"
                )
            });
    let result = limits.0.general.check_key(&ip).and_then(|()| {
        if auth_request {
            limits.0.auth.check_key(&ip)
        } else {
            Ok(())
        }
    });
    if let Err(denied) = result {
        let wait = denied.wait_time_from(DefaultClock::default().now());
        let seconds = wait
            .as_secs()
            .saturating_add(u64::from(wait.subsec_nanos() > 0))
            .max(1);
        let mut response = ApiProblem::new(ProblemKind::RateLimitExceeded, context).into_response();
        response.headers_mut().insert(
            header::RETRY_AFTER,
            seconds.to_string().parse().expect("integer header"),
        );
        return response;
    }
    next.run(request).await
}

fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        ip => ip,
    }
}

// Fail closed to the peer bucket on any ambiguity; never salvage a partial chain.
fn client_ip(peer: IpAddr, headers: &axum::http::HeaderMap, trusted: &[ipnet::IpNet]) -> IpAddr {
    let peer = canonical_ip(peer);
    let is_trusted = |ip: IpAddr| trusted.iter().any(|net| net.contains(&ip));
    if !is_trusted(peer) {
        return peer;
    }
    let mut values = headers.get_all("x-forwarded-for").iter();
    let Some(value) = values.next() else {
        return peer;
    };
    if values.next().is_some() || value.as_bytes().len() > 4096 {
        return peer;
    }
    let Ok(value) = value.to_str() else {
        return peer;
    };
    let mut chain = [peer; 32];
    let mut count = 0;
    for hop in value.split(',') {
        if count == chain.len() {
            return peer;
        }
        let Ok(ip) = hop.trim_matches([' ', '\t']).parse::<IpAddr>() else {
            return peer;
        };
        chain[count] = canonical_ip(ip);
        count += 1;
    }
    chain[..count]
        .iter()
        .rev()
        .copied()
        .find(|ip| !is_trusted(*ip))
        .unwrap_or(peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn trust_is_opt_in_and_chain_is_walked_from_the_peer() {
        let trusted = ["127.0.0.0/8".parse().unwrap(), "fd00::/8".parse().unwrap()];
        let peer = "127.0.0.1".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "192.0.2.99, 198.51.100.1, fd00::1".parse().unwrap(),
        );
        assert_eq!(client_ip(peer, &headers, &[]), peer);
        assert_eq!(
            client_ip("203.0.113.1".parse().unwrap(), &headers, &trusted),
            "203.0.113.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            client_ip(peer, &headers, &trusted),
            "198.51.100.1".parse::<IpAddr>().unwrap()
        );
        headers.insert("x-forwarded-for", "2001:db8::1, 127.0.0.2".parse().unwrap());
        assert_eq!(
            client_ip(peer, &headers, &trusted),
            "2001:db8::1".parse::<IpAddr>().unwrap()
        );
        headers.insert("x-forwarded-for", "::ffff:198.51.100.1".parse().unwrap());
        assert_eq!(
            client_ip("::ffff:127.0.0.1".parse().unwrap(), &headers, &trusted),
            "198.51.100.1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn malformed_or_ambiguous_chains_fall_back_without_partial_parsing() {
        let peer = "127.0.0.1".parse().unwrap();
        let trusted = ["127.0.0.0/8".parse().unwrap()];
        let mut headers = HeaderMap::new();
        assert_eq!(client_ip(peer, &headers, &trusted), peer);
        for value in [
            "",
            "unknown",
            "198.51.100.1,",
            "bad, 198.51.100.1",
            "198.51.100.1:80",
            "[2001:db8::1]",
            "127.0.0.2",
            "198.51.100.1, 127.0.0.2, bad",
        ] {
            headers.insert("x-forwarded-for", value.parse().unwrap());
            assert_eq!(client_ip(peer, &headers, &trusted), peer, "{value}");
        }
        for value in [
            vec!["198.51.100.1"; 33].join(","),
            format!("{}198.51.100.1", " ".repeat(4096)),
        ] {
            headers.insert("x-forwarded-for", value.parse().unwrap());
            assert_eq!(client_ip(peer, &headers, &trusted), peer);
        }
        headers.insert("x-forwarded-for", HeaderValue::from_bytes(b"\xff").unwrap());
        assert_eq!(client_ip(peer, &headers, &trusted), peer);
        headers.insert("x-forwarded-for", "198.51.100.1".parse().unwrap());
        headers.append("x-forwarded-for", "198.51.100.2".parse().unwrap());
        assert_eq!(client_ip(peer, &headers, &trusted), peer);
        headers.remove("x-forwarded-for");
        headers.insert("forwarded", "for=198.51.100.1".parse().unwrap());
        headers.insert("x-real-ip", "198.51.100.1".parse().unwrap());
        assert_eq!(client_ip(peer, &headers, &trusted), peer);
    }

    #[test]
    fn accepts_exact_hop_and_byte_boundaries() {
        let peer = "127.0.0.1".parse().unwrap();
        let client = "198.51.100.1".parse::<IpAddr>().unwrap();
        let trusted = ["127.0.0.1/32".parse().unwrap()];
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            vec!["198.51.100.1"; 32].join(",").parse().unwrap(),
        );
        assert_eq!(client_ip(peer, &headers, &trusted), client);
        let value = format!("{}198.51.100.1", " ".repeat(4096 - "198.51.100.1".len()));
        headers.insert("x-forwarded-for", value.parse().unwrap());
        assert_eq!(client_ip(peer, &headers, &trusted), client);
    }
}
