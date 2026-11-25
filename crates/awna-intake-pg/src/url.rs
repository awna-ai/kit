//! Local-only connection option validation.

use std::error::Error;
use std::fmt;
use std::str::FromStr;

use sqlx::ConnectOptions;
use sqlx::postgres::{PgConnectOptions, PgSslMode};

/// A database URL that violates a fixed connection rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UrlError(&'static str);

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Error for UrlError {}

/// Parse a database URL and refuse remote targets and required TLS.
///
/// Remote databases must be reached through a local tunnel or Unix socket.
/// Errors never include any part of the input URL.
pub fn connect_options(url: &str) -> Result<PgConnectOptions, UrlError> {
    // SQLx logs unknown URL parameters with their values. Check the keys using
    // its URL type before invoking its options parser, without adding a URL crate.
    let parsed = parse_like(
        PgConnectOptions::new_without_pgpass()
            .host("localhost")
            .username("check")
            .password("")
            .database("check")
            .to_url_lossy(),
        url,
    )?;
    for (key, _) in parsed.query_pairs() {
        if !recognized_parameter(&key) {
            return Err(UrlError("database URL parameter is unsupported"));
        }
    }
    let options =
        PgConnectOptions::from_str(url).map_err(|_| UrlError("database URL is invalid"))?;
    let local_host = matches!(
        options.get_host(),
        "localhost" | "127.0.0.1" | "::1" | "[::1]"
    );
    let local_socket = options.get_socket().is_some_and(|path| path.is_absolute());
    if !local_host && !local_socket {
        return Err(UrlError(
            "database target must be loopback or an absolute Unix socket",
        ));
    }
    if options.get_socket().is_some_and(|path| !path.is_absolute()) {
        return Err(UrlError("database socket must be absolute"));
    }
    // SQLx exposes no hostaddr getter. Its public URL conversion includes the
    // effective value, including PGHOSTADDR. Replace user-controlled credentials
    // first because its URL builder interpolates the username without escaping it.
    let inspection = options
        .clone()
        .host("localhost")
        .username("check")
        .password("")
        .database("check")
        .to_url_lossy();
    for (key, value) in inspection.query_pairs() {
        if key == "hostaddr"
            && !value
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        {
            return Err(UrlError("database hostaddr must be loopback"));
        }
    }
    if matches!(
        options.get_ssl_mode(),
        PgSslMode::Require | PgSslMode::VerifyCa | PgSslMode::VerifyFull
    ) {
        return Err(UrlError("this build has no TLS"));
    }
    Ok(options)
}

// Infer SQLx's public URL type from its existing conversion API.
fn parse_like<T: FromStr>(_: T, input: &str) -> Result<T, UrlError> {
    input
        .parse()
        .map_err(|_| UrlError("database URL is invalid"))
}

fn recognized_parameter(key: &str) -> bool {
    matches!(
        key,
        "sslmode"
            | "ssl-mode"
            | "sslrootcert"
            | "ssl-root-cert"
            | "ssl-ca"
            | "sslcert"
            | "ssl-cert"
            | "sslkey"
            | "ssl-key"
            | "statement-cache-capacity"
            | "host"
            | "hostaddr"
            | "port"
            | "dbname"
            | "user"
            | "password"
            | "application_name"
            | "options"
    ) || key.starts_with("options[")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_local_targets() {
        for (index, url) in [
            "postgres://someone@localhost/none",
            "postgres://a%40b:pw-SENTINEL-7f3a@localhost/none",
            "postgres://someone@127.0.0.1/none",
            "postgres://someone@[::1]/none",
            "postgres://someone@localhost/none?host=::1",
            "postgres://someone@%2Ftmp/none",
            "postgres://someone@localhost/none?host=/tmp",
            "postgres://someone@localhost/none?hostaddr=127.0.0.2",
            "postgres://someone@localhost/none?hostaddr=::1",
        ]
        .into_iter()
        .enumerate()
        {
            assert!(connect_options(url).is_ok(), "local target {index} refused");
        }
    }

    #[test]
    fn refuses_remote_targets_and_required_tls_without_echoing_input() {
        for suffix in [
            "db.example.com/none",
            "10.0.0.5/none",
            "localhost/none?hostaddr=10.0.0.5",
            "localhost/none?sslmode=require",
            "localhost/none?sslmode=verify-ca",
            "localhost/none?sslmode=verify-full",
            "localhost/none?host=db.example.com",
            "localhost/none?host=relative",
            "localhost/none?unrecognized=pw-SENTINEL-7f3a",
        ] {
            let url = format!("postgres://someone:pw-SENTINEL-7f3a@{suffix}");
            let Err(error) = connect_options(&url) else {
                panic!("unsafe target accepted");
            };
            let rendered = format!("{error} {error:?}");
            for secret in ["someone", "SENTINEL", "db.example.com", "10.0.0.5", "none"] {
                assert!(!rendered.contains(secret));
            }
        }
    }

    #[test]
    fn invalid_urls_have_a_fixed_error() {
        assert_eq!(
            connect_options("not a URL")
                .expect_err("invalid URL accepted")
                .to_string(),
            "database URL is invalid"
        );
    }
}
