//! The session token interface an account system plugs into.
//!
//! A client presents an opaque session token in its handshake; the server
//! hands it to an `Authenticator` and either gets the account it names or a
//! refusal. No accounts, passwords or personal data exist here: the only
//! implementation is `DevTokens`, which accepts `dev:<name>` tokens for local
//! play and tests. A real deployment implements `Authenticator` over tokens
//! issued by the account service (for example a signed, expiring token
//! checked against the service's public key, with no call per login), and
//! moves the handshake under encryption (see the report's plan).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// Stable account id (the persistence key later).
    pub id: u64,
    /// What other players see.
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Malformed,
    Expired,
    Unknown,
    Banned,
}

pub trait Authenticator: Send + Sync {
    fn validate(&self, token: &[u8]) -> Result<Account, Refusal>;
}

/// Development tokens: `dev:<name>` (1-24 printable ASCII characters). The
/// account id is a hash of the name, so the same name is the same account.
pub struct DevTokens;

impl Authenticator for DevTokens {
    fn validate(&self, token: &[u8]) -> Result<Account, Refusal> {
        let s = std::str::from_utf8(token).map_err(|_| Refusal::Malformed)?;
        let name = s.strip_prefix("dev:").ok_or(Refusal::Unknown)?;
        if name.is_empty() || name.len() > 24 || !name.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(Refusal::Malformed);
        }
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in name.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
        Ok(Account { id: h, display_name: name.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dev_tokens() {
        assert_eq!(DevTokens.validate(b"dev:ada").unwrap().display_name, "ada");
        assert_eq!(DevTokens.validate(b"dev:ada").unwrap().id, DevTokens.validate(b"dev:ada").unwrap().id);
        assert_eq!(DevTokens.validate(b"ada"), Err(Refusal::Unknown));
        assert_eq!(DevTokens.validate(b"dev:"), Err(Refusal::Malformed));
    }
}
