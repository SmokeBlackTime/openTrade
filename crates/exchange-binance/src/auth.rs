use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Sign a query string with HMAC-SHA256.
pub fn sign_query(query: &str, secret: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(query.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_signature_deterministic() {
        let sig1 = sign_query("symbol=BTCUSDT&timestamp=1234567890", "mysecret");
        let sig2 = sign_query("symbol=BTCUSDT&timestamp=1234567890", "mysecret");
        assert_eq!(sig1, sig2);
        assert!(!sig1.is_empty());
    }

    #[test]
    fn different_secrets_produce_different_signatures() {
        let sig1 = sign_query("test", "secret1");
        let sig2 = sign_query("test", "secret2");
        assert_ne!(sig1, sig2);
    }
}
