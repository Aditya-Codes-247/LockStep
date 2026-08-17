use url::Url;

/// A whitelist entry: a host (optionally with a port) that is allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRule {
    pub host: String,
    pub port: Option<u16>,
}

impl HostRule {
    /// Parse an entry like `"coinswitch.co"` or `"example.com:8080"`.
    /// Lowercases, strips optional leading `*` / whitespace, and removes a
    /// trailing dot (fully-qualified hostname form).
    pub fn parse(raw: &str) -> Option<HostRule> {
        let s = raw.trim().trim_start_matches('*').trim();
        if s.is_empty() {
            return None;
        }
        let s = s.to_lowercase();
        let s = s.trim_end_matches('.').to_string();
        let (host, port) = match s.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() => {
                let port: u16 = p.parse().ok()?;
                (h.to_string(), Some(port))
            }
            _ => (s.clone(), None),
        };
        if host.is_empty() || host.contains('/') || host.contains(' ') {
            return None;
        }
        Some(HostRule { host, port })
    }

    /// Whether an absolute URL's host satisfies this rule.
    /// Matches the exact host or any subdomain of the rule host.
    /// A port is only enforced when the rule declares one.
    pub fn matches(&self, url: &Url) -> bool {
        if url.scheme() != "http" && url.scheme() != "https" {
            return false;
        }
        let Some(host) = url.host_str() else {
            return false;
        };
        let host = host.to_lowercase();
        let host = host.trim_end_matches('.');
        let host_ok = host == self.host || host.ends_with(&format!(".{}", self.host));
        if !host_ok {
            return false;
        }
        match self.port {
            Some(rule_port) => {
                let actual = url.port().or_else(|| match url.scheme() {
                    "https" => Some(443),
                    "http" => Some(80),
                    _ => None,
                });
                actual == Some(rule_port)
            }
            None => true,
        }
    }
}

/// Normalize the raw user input in the address bar into an absolute URL.
///
/// - Leaves `http(s)://` URLs as-is.
/// - Prepends `https://` when there is no scheme.
/// - Rejects opaque/non-hierarchical schemes.
pub fn normalize_input(raw: &str) -> Result<Url, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Please enter a web address.".into());
    }
    let with_scheme = if trimmed.contains("://") || trimmed.starts_with("about:") {
        trimmed.to_string()
    } else {
        format!("https://{}", trimmed)
    };
    let url = Url::parse(&with_scheme).map_err(|_| format!("'{trimmed}' is not a valid web address."))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(format!("Scheme '{other}:' is not supported. Only http and https are allowed.")),
    }
}

/// Determine the block reason for a URL, or `None` if it is allowed.
pub fn block_reason(url: &Url, rules: &[HostRule]) -> Option<String> {
    if url.scheme() != "http" && url.scheme() != "https" {
        return Some(format!("Scheme '{}:' is not allowed.", url.scheme()));
    }
    let Some(host) = url.host_str() else {
        return Some("The address does not point to a website.".into());
    };
    if rules.iter().any(|r| r.matches(url)) {
        return None;
    }
    Some(format!(
        "{} is not on the approved website list.",
        host.to_lowercase()
    ))
}

/// Parse a list of raw whitelist entries into rules.
pub fn parse_rules(entries: &[String]) -> Vec<HostRule> {
    entries.iter().filter_map(|e| HostRule::parse(e)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn normalizes_bare_domain_to_https() {
        let u = normalize_input("coinswitch.co").unwrap();
        assert_eq!(u.host_str(), Some("coinswitch.co"));
        assert_eq!(u.scheme(), "https");
    }

    #[test]
    fn rejects_bad_schemes() {
        assert!(normalize_input("javascript:alert(1)").is_err());
        assert!(normalize_input("data:text/html,hello").is_err());
        assert!(normalize_input("file:///etc/passwd").is_err());
        assert!(normalize_input("").is_err());
    }

    #[test]
    fn exact_host_matches() {
        let rules = parse_rules(&["dhan.co".into(), "coinswitch.co:443".into()]);
        assert_eq!(block_reason(&url("https://dhan.co/"), &rules), None);
        assert_eq!(block_reason(&url("https://www.dhan.co/trade"), &rules), None);
        assert_eq!(block_reason(&url("http://dhan.co:8080/"), &rules), None);
        assert_eq!(block_reason(&url("https://coinswitch.co/"), &rules), None);
        assert_eq!(block_reason(&url("https://coinswitch.co/trade"), &rules), None);
    }

    #[test]
    fn subdomains_are_allowed_but_similar_names_are_not() {
        let rules = parse_rules(&["dhan.co".into()]);
        assert_eq!(block_reason(&url("https://tv.dhan.co/"), &rules), None);
        assert_eq!(block_reason(&url("https://a.b.dhan.co/"), &rules), None);
        assert!(block_reason(&url("https://dhan.co.evil.com/"), &rules).is_some());
        assert!(block_reason(&url("https://notdhan.co/"), &rules).is_some());
        assert!(block_reason(&url("https://dhan.co.org/"), &rules).is_some());
    }

    #[test]
    fn port_rule_is_enforced() {
        let rules = parse_rules(&["example.com:8080".into()]);
        assert_eq!(block_reason(&url("https://example.com:8080/"), &rules), None);
        assert!(block_reason(&url("https://example.com/"), &rules).is_some());
        assert!(block_reason(&url("https://example.com:8443/"), &rules).is_some());
    }

    #[test]
    fn scheme_is_always_enforced() {
        let rules = parse_rules(&["example.com".into()]);
        assert!(block_reason(&url("ftp://example.com/"), &rules).is_some());
    }

    #[test]
    fn fqdn_trailing_dot_parses() {
        let rules = parse_rules(&["dhan.co.".into()]);
        assert_eq!(block_reason(&url("https://www.dhan.co/"), &rules), None);
    }
}