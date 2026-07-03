//! SSRF guard (security audit #5).
//!
//! A writer authors the URLs/hosts the worker fleet connects to. Without a guard
//! a writer (or anyone who obtains a writer token) could point a check at an
//! internal-only service or the cloud metadata endpoint (`169.254.169.254`) and
//! read timing/response signal back through the metrics — using the monitoring
//! fleet as an SSRF pivot.
//!
//! The fleet legitimately monitors internal endpoints too, so a blanket deny is
//! wrong. The policy is: an internal (loopback / link-local / private / ULA)
//! resolved address is refused unless the endpoint sets `allow_internal` AND the
//! resolved address falls inside an operator-configured allowlist CIDR. Public
//! addresses are always permitted. The check runs at connect time against the
//! *resolved* address, which also defends against DNS rebinding — a hostname that
//! resolves to a public address at validation time but an internal one at connect
//! time is still caught.

use std::net::IpAddr;

use ipnet::IpNet;

/// Whether `ip` is an internal / non-routable address that must not be reached
/// without an explicit opt-in. Covers loopback, private (RFC1918 / ULA),
/// link-local (including the `169.254.169.254` metadata address), and the
/// unspecified address, for both IPv4 and IPv6.
pub fn is_internal_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                // Carrier-grade NAT 100.64.0.0/10 — not publicly routable.
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 0x40)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // Unique local addresses fc00::/7.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // Link-local unicast fe80::/10.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // Map v4-mapped/compatible back to their v4 classification.
                || v6.to_ipv4_mapped().map(IpAddr::V4).is_some_and(is_internal_ip)
        }
    }
}

/// Why a resolved address was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuardError {
    /// The address is internal and the endpoint did not opt in.
    #[error("address {ip} is internal; set allow_internal to reach it")]
    InternalNotAllowed {
        /// The refused address.
        ip: IpAddr,
    },
    /// The endpoint opted in, but the address is not within any operator-approved
    /// allowlist CIDR.
    #[error("address {ip} is internal and not in the operator allowlist")]
    NotInAllowlist {
        /// The refused address.
        ip: IpAddr,
    },
}

/// The operator's network policy: which internal CIDRs an opted-in endpoint may
/// reach. Public addresses are always allowed regardless of this list.
#[derive(Debug, Clone, Default)]
pub struct NetPolicy {
    allowlist: Vec<IpNet>,
}

/// Failure parsing an operator allowlist entry.
#[derive(Debug, thiserror::Error)]
#[error("invalid allowlist CIDR {cidr:?}: {source}")]
pub struct PolicyParseError {
    /// The offending entry.
    pub cidr: String,
    #[source]
    source: ipnet::AddrParseError,
}

impl NetPolicy {
    /// Build a policy from operator-supplied allowlist CIDRs (e.g.
    /// `["10.0.0.0/8", "192.168.0.0/16"]`). A bare IP without a prefix is
    /// accepted as a host route (`/32` or `/128`).
    pub fn new<S: AsRef<str>>(cidrs: &[S]) -> Result<Self, PolicyParseError> {
        let mut allowlist = Vec::with_capacity(cidrs.len());
        for entry in cidrs {
            let entry = entry.as_ref().trim();
            let net: IpNet = match entry.parse::<IpNet>() {
                Ok(net) => net,
                // Allow a bare address by promoting it to a host route; only if
                // that also fails do we surface the (richer) CIDR parse error.
                Err(net_err) => match entry.parse::<IpAddr>() {
                    Ok(ip) => IpNet::from(ip),
                    Err(_) => {
                        return Err(PolicyParseError {
                            cidr: entry.to_string(),
                            source: net_err,
                        })
                    }
                },
            };
            allowlist.push(net);
        }
        Ok(Self { allowlist })
    }

    /// Whether the allowlist covers `ip`.
    fn allows(&self, ip: IpAddr) -> bool {
        self.allowlist.iter().any(|net| net.contains(&ip))
    }

    /// Decide whether a connection to the resolved address `ip` is permitted.
    /// `allow_internal` is the endpoint's opt-in flag. Public addresses always
    /// pass; internal addresses pass only when opted in and allowlisted.
    pub fn guard(&self, ip: IpAddr, allow_internal: bool) -> Result<(), GuardError> {
        if !is_internal_ip(ip) {
            return Ok(());
        }
        if !allow_internal {
            return Err(GuardError::InternalNotAllowed { ip });
        }
        if !self.allows(ip) {
            return Err(GuardError::NotInAllowlist { ip });
        }
        Ok(())
    }
}
