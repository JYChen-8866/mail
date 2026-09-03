use crate::{
    error::{CoreError, Result},
    models::Provider,
    oauth::loopback::{AuthCode, LoopbackServer},
};
use async_trait::async_trait;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Access token returned by a platform authorization service such as Google
/// Play Services. Native mobile authorization does not expose a reusable
/// refresh token to the application; the platform must be asked for a fresh
/// access token when this one expires.
pub struct PlatformAuthorization {
    pub access_token: String,
    pub expires_in: Option<i64>,
}

#[async_trait]
pub trait OAuthRedirectSession: Send {
    fn redirect_uri(&self) -> &str;
    async fn wait(self: Box<Self>, timeout: Duration) -> Result<AuthCode>;
}

#[async_trait]
pub trait OAuthRedirectBroker: Send + Sync {
    async fn begin(
        &self,
        provider: Provider,
        expected_state: &str,
    ) -> Result<Box<dyn OAuthRedirectSession>>;

    /// Use a platform-native authorization API when one exists. `None` means
    /// this broker only supports browser redirects. A non-interactive request
    /// must return `NeedsReauth` rather than opening account or consent UI.
    async fn authorize_platform(
        &self,
        _provider: Provider,
        _scopes: &[&str],
        _interactive: bool,
    ) -> Option<Result<PlatformAuthorization>> {
        None
    }
}

pub type OAuthRedirectBrokerHandle = Arc<dyn OAuthRedirectBroker>;

/// Single-use, expiring verifier shared by redirect brokers. A rejected
/// callback does not consume the transaction, but the first valid callback
/// does, so browser retries cannot redeem the same request twice.
pub struct OAuthRedirectGuard {
    expected_state: String,
    expires_at: Instant,
    consumed: bool,
}

impl OAuthRedirectGuard {
    pub fn new(expected_state: impl Into<String>, lifetime: Duration) -> Self {
        Self {
            expected_state: expected_state.into(),
            expires_at: Instant::now() + lifetime,
            consumed: false,
        }
    }

    pub fn accept(&mut self, uri: &str) -> Result<AuthCode> {
        if self.consumed {
            return Err(CoreError::Auth(
                "OAuth callback is expired or already used".into(),
            ));
        }
        if Instant::now() >= self.expires_at {
            return Err(CoreError::Auth("OAuth callback has expired".into()));
        }
        let response = parse_redirect_uri(uri)?;
        if response.state.as_deref() != Some(self.expected_state.as_str()) {
            return Err(CoreError::Auth("oauth state mismatch".into()));
        }
        self.consumed = true;
        Ok(response)
    }
}

pub fn parse_redirect_uri(uri: &str) -> Result<AuthCode> {
    let uri = url::Url::parse(uri)
        .map_err(|_| CoreError::Auth("OAuth redirect URI is invalid".into()))?;
    let mut code = None;
    let mut state = None;
    let mut provider_error = None;
    for (key, value) in uri.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => provider_error = Some(value.into_owned()),
            _ => {}
        }
    }
    if let Some(error) = provider_error {
        return Err(CoreError::Auth(format!("oauth error: {error}")));
    }
    Ok(AuthCode {
        code: code.ok_or_else(|| CoreError::Auth("OAuth redirect has no code".into()))?,
        state,
    })
}

#[derive(Default)]
pub struct LoopbackRedirectBroker;

struct LoopbackRedirectSession {
    redirect_uri: String,
    server: LoopbackServer,
}

#[async_trait]
impl OAuthRedirectBroker for LoopbackRedirectBroker {
    async fn begin(
        &self,
        provider: Provider,
        _expected_state: &str,
    ) -> Result<Box<dyn OAuthRedirectSession>> {
        let server = LoopbackServer::bind().await?;
        let redirect_uri = match provider {
            Provider::Gmail => server.ipv4_redirect_uri(),
            Provider::Microsoft => server.localhost_redirect_uri(),
            Provider::Imap => return Err(CoreError::Auth("provider does not use oauth".into())),
        };
        Ok(Box::new(LoopbackRedirectSession {
            redirect_uri,
            server,
        }))
    }
}

#[async_trait]
impl OAuthRedirectSession for LoopbackRedirectSession {
    fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    async fn wait(self: Box<Self>, timeout: Duration) -> Result<AuthCode> {
        self.server.wait_for_code(timeout).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_parser_decodes_code_and_state_and_rejects_denial() {
        assert_eq!(
            parse_redirect_uri("com.flectar.mail.oauth://callback?code=a%2Fb&state=ok").unwrap(),
            AuthCode {
                code: "a/b".into(),
                state: Some("ok".into())
            }
        );
        assert!(
            parse_redirect_uri("com.flectar.mail.oauth://callback?error=access_denied&state=ok")
                .is_err()
        );
    }

    #[test]
    fn redirect_guard_rejects_wrong_state_expiry_and_duplicate_delivery() {
        let mut guard = OAuthRedirectGuard::new("right", Duration::from_secs(300));
        assert!(
            guard
                .accept("com.flectar.mail.oauth://callback?code=nope&state=wrong")
                .unwrap_err()
                .to_string()
                .contains("state mismatch")
        );
        assert_eq!(
            guard
                .accept("com.flectar.mail.oauth://callback?code=usable&state=right")
                .unwrap()
                .code,
            "usable"
        );
        assert!(
            guard
                .accept("com.flectar.mail.oauth://callback?code=again&state=right")
                .unwrap_err()
                .to_string()
                .contains("already used")
        );

        let mut expired = OAuthRedirectGuard::new("right", Duration::ZERO);
        assert!(
            expired
                .accept("com.flectar.mail.oauth://callback?code=late&state=right")
                .unwrap_err()
                .to_string()
                .contains("expired")
        );
    }
}
