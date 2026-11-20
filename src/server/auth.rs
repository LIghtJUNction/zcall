//! Private inbound credentials, independent of every upstream provider key.
//!
//! The JSON format remains compatible with zcalld caller_keys files. Serialization
//! contains secrets and is only for a host-owned private store. No operation
//! generates weak randomness, logs a secret, or silently writes a file.
use super::http::{self, Request};
use crate::api::json;
use std::{collections::BTreeSet, fs, io::Read, path::Path};

pub const MAX_KEYS: usize = 256;
pub const MAX_FILE_BYTES: usize = 256 * 1024;

#[derive(Clone, Default)]
pub struct CallerKeys {
    keys: Vec<ApiKey>,
}

/// A managed key. Debug formatting is intentionally unavailable.
#[derive(Clone)]
pub struct ApiKey {
    id: Option<String>,
    name: Option<String>,
    secret: String,
    lan: bool,
    workspace: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KeyMetadata<'a> {
    pub id: Option<&'a str>,
    pub name: Option<&'a str>,
    pub lan: bool,
    pub workspace: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authentication {
    ApiKey,
    SharedToken,
    TrustedLoopback,
    Unrestricted,
}

/// Contains only verified, server-owned metadata; never the presented secret.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedCaller<'a> {
    pub key_id: Option<&'a str>,
    pub workspace: Option<&'a str>,
    pub authentication: Authentication,
}

pub struct AuthPolicy<'a> {
    pub keys: Option<&'a CallerKeys>,
    pub token: Option<&'a str>,
    pub trust_loopback: bool,
}

fn valid_text(value: &str, maximum: usize) -> bool {
    value.len() <= maximum && !value.chars().any(char::is_control)
}

impl ApiKey {
    /// The caller supplies a secret from its established secure random source.
    pub fn new(id: impl Into<String>, secret: impl Into<String>) -> Result<Self, String> {
        let key = Self {
            id: Some(id.into()),
            name: None,
            secret: secret.into(),
            lan: false,
            workspace: None,
        };
        key.validate()?;
        Ok(key)
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Result<Self, String> {
        self.name = Some(name.into());
        self.validate()?;
        Ok(self)
    }

    pub fn with_lan(mut self, allowed: bool) -> Self {
        self.lan = allowed;
        self
    }

    pub fn with_workspace(mut self, path: impl Into<String>) -> Result<Self, String> {
        self.workspace = Some(path.into());
        self.validate()?;
        Ok(self)
    }

    pub fn metadata(&self) -> KeyMetadata<'_> {
        KeyMetadata {
            id: self.id.as_deref(),
            name: self.name.as_deref(),
            lan: self.lan,
            workspace: self.workspace.as_deref(),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self
            .id
            .as_ref()
            .is_some_and(|value| value.is_empty() || !valid_text(value, 512))
            || self
                .name
                .as_ref()
                .is_some_and(|value| !valid_text(value, 512))
        {
            return Err("caller key metadata is invalid".into());
        }
        if self.secret.is_empty()
            || self.secret.len() > 16384
            || self.secret.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err("caller key secret is empty or invalid".into());
        }
        if let Some(workspace) = &self.workspace
            && (!valid_text(workspace, 4096)
                || !Path::new(workspace).is_absolute()
                || self.id.is_none())
        {
            return Err("caller workspace needs an absolute path and an explicit key id".into());
        }
        Ok(())
    }
}

impl CallerKeys {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        parse(bytes)
    }

    pub fn entries(&self) -> impl ExactSizeIterator<Item = KeyMetadata<'_>> {
        self.keys.iter().map(ApiKey::metadata)
    }

    /// Insert only a new identifier/secret; a failed mutation preserves all keys.
    pub fn insert(&mut self, key: ApiKey) -> Result<(), String> {
        key.validate()?;
        if self.keys.len() >= MAX_KEYS {
            return Err("caller keys file exceeds 256 keys".into());
        }
        if self.keys.iter().any(|existing| {
            (key.id.is_some() && key.id == existing.id)
                || http::constant_eq(key.secret.as_bytes(), existing.secret.as_bytes())
        }) {
            return Err("caller key identifiers and secrets must be unique".into());
        }
        self.keys.push(key);
        Ok(())
    }

    /// Replace one existing secret. Existing authenticated requests are not cancelled.
    pub fn rotate(&mut self, id: &str, secret: impl Into<String>) -> Result<(), String> {
        let index = self
            .keys
            .iter()
            .position(|key| key.id.as_deref() == Some(id))
            .ok_or("caller key identifier not found")?;
        let mut replacement = self.keys[index].clone();
        replacement.secret = secret.into();
        replacement.validate()?;
        if self.keys.iter().enumerate().any(|(other, key)| {
            other != index
                && http::constant_eq(key.secret.as_bytes(), replacement.secret.as_bytes())
        }) {
            return Err("caller key secrets must be unique".into());
        }
        self.keys[index] = replacement;
        Ok(())
    }

    /// Revoke an explicit identifier for subsequent authentication checks.
    pub fn revoke(&mut self, id: &str) -> bool {
        if let Some(index) = self
            .keys
            .iter()
            .position(|key| key.id.as_deref() == Some(id))
        {
            self.keys.remove(index);
            true
        } else {
            false
        }
    }

    /// Examine every configured key even after a match.
    pub fn verify(&self, supplied: &str, loopback: bool) -> Option<VerifiedCaller<'_>> {
        let mut verified = None;
        for key in &self.keys {
            let matches = http::constant_eq(supplied.as_bytes(), key.secret.as_bytes());
            if matches & (loopback || key.lan) {
                verified = Some(VerifiedCaller {
                    key_id: key.id.as_deref(),
                    workspace: key.workspace.as_deref(),
                    authentication: Authentication::ApiKey,
                });
            }
        }
        verified
    }

    pub fn id(&self, supplied: &str, loopback: bool) -> Option<&str> {
        self.verify(supplied, loopback)
            .and_then(|caller| caller.key_id)
    }

    pub fn workspace(&self, supplied: &str, loopback: bool) -> Option<&str> {
        self.verify(supplied, loopback)
            .and_then(|caller| caller.workspace)
    }

    pub fn allows_lan(&self) -> bool {
        self.keys.iter().any(|key| key.lan)
    }

    pub fn permits(&self, supplied: &str, loopback: bool) -> bool {
        self.verify(supplied, loopback).is_some()
    }

    /// Secret-bearing JSON for the host's private atomic persistence operation.
    /// This preserves the existing caller_keys array schema; it does not write a file.
    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        let mut body = String::from("[");
        for (index, key) in self.keys.iter().enumerate() {
            if index != 0 {
                body.push(',');
            }
            body.push('{');
            body.push_str("\"secret\":");
            body.push_str(&http::quote(&key.secret));
            body.push_str(if key.lan {
                ",\"lan\":true"
            } else {
                ",\"lan\":false"
            });
            for (name, value) in [
                ("id", key.id.as_deref()),
                ("name", key.name.as_deref()),
                ("workspace", key.workspace.as_deref()),
            ] {
                if let Some(value) = value {
                    body.push(',');
                    body.push_str(&http::quote(name));
                    body.push(':');
                    body.push_str(&http::quote(value));
                }
            }
            body.push('}');
            if body.len() > MAX_FILE_BYTES - 2 {
                return Err("caller keys file exceeds 256 KiB".into());
            }
        }
        body.push_str("]\n");
        Ok(body.into_bytes())
    }
}

/// Header precedence matches the existing gateway's credential contract.
pub fn supplied_key(request: &Request) -> Option<&str> {
    request
        .header("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .or_else(|| request.header("x-api-key"))
        .or_else(|| request.header("x-goog-api-key"))
}

pub fn authenticate<'a>(
    request: &Request,
    policy: &AuthPolicy<'a>,
    loopback: bool,
) -> Option<VerifiedCaller<'a>> {
    let supplied = supplied_key(request);
    // A valid key retains its workspace binding even when local trust is enabled.
    if let Some(caller) = supplied.and_then(|value| policy.keys?.verify(value, loopback)) {
        return Some(caller);
    }
    let authentication = if policy.trust_loopback && loopback {
        Authentication::TrustedLoopback
    } else if policy.keys.is_none() && policy.token.is_none() {
        Authentication::Unrestricted
    } else if supplied
        .zip(policy.token)
        .is_some_and(|(value, token)| http::constant_eq(value.as_bytes(), token.as_bytes()))
    {
        Authentication::SharedToken
    } else {
        return None;
    };
    Some(VerifiedCaller {
        key_id: None,
        workspace: None,
        authentication,
    })
}

/// Read the existing private caller_keys file with a bounded allocation.
pub fn load(path: &Path) -> Result<CallerKeys, String> {
    let file = fs::File::open(path).map_err(|error| format!("caller keys file: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("caller keys file: {error}"))?;
    if !metadata.is_file() {
        return Err("caller keys path must be a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("caller keys file must have mode 0600".into());
        }
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("caller keys file: {error}"))?;
    parse(&bytes)
}

fn text(value: json::Value<'_, '_>) -> Result<String, String> {
    let string = value
        .as_string()
        .ok_or("caller key metadata must be strings")?;
    String::from_utf8(string.bytes().collect()).map_err(|_| "invalid caller key string".into())
}

pub fn parse(bytes: &[u8]) -> Result<CallerKeys, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("caller keys file exceeds 256 KiB".into());
    }
    let mut tokens = vec![json::Token::EMPTY; 4096];
    let document = json::parse(
        bytes,
        &mut tokens,
        json::Limits {
            max_depth: 4,
            max_input: MAX_FILE_BYTES,
            max_tokens: 4096,
            max_object_fields: 5,
        },
    )
    .map_err(|_| "caller keys file is invalid JSON")?;
    let root = document.root();
    if root.kind() != json::Kind::Array {
        return Err("caller keys file must contain an array".into());
    }
    let mut keys = CallerKeys::new();
    let mut ids = BTreeSet::new();
    for entry in root.array() {
        if entry.kind() != json::Kind::Object {
            return Err("caller key must be an object".into());
        }
        if entry.object().any(|(name, _)| {
            !["id", "name", "lan", "secret", "workspace"]
                .iter()
                .any(|known| name.eq_str(known))
        }) {
            return Err("caller key contains an unknown field".into());
        }
        let id = entry.get("id").map(text).transpose()?;
        let name = entry.get("name").map(text).transpose()?;
        if let Some(id) = &id
            && (id.is_empty() || !ids.insert(id.clone()))
        {
            return Err("caller key identifiers must be nonempty and unique".into());
        }
        let secret = text(
            entry
                .get("secret")
                .ok_or("caller key secret must be a string")?,
        )?;
        let lan = match entry.get("lan") {
            None => false,
            Some(value) if value.is_null() => false,
            Some(value) => value.as_bool().ok_or("caller key lan must be a boolean")?,
        };
        let workspace = match entry.get("workspace") {
            None => None,
            Some(value) if value.is_null() => None,
            Some(value) => Some(text(value)?),
        };
        keys.insert(ApiKey {
            id,
            name,
            secret,
            lan,
            workspace,
        })?;
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(header: &str, secret: &str) -> Request {
        Request {
            method: "POST".into(),
            path: "/v1/chat/completions".into(),
            headers: vec![(header.into(), secret.into())],
            body: Vec::new(),
        }
    }

    #[test]
    fn compatible_private_schema_preserves_scope_metadata_and_escapes() {
        let keys = parse(br#"[{"id":"local","name":"A \"label\"","secret":"local-key","lan":false},{"id":"network","secret":"network-key","lan":true},{"secret":"anonymous-key"}]"#).unwrap();
        assert!(keys.permits("local-key", true));
        assert!(!keys.permits("local-key", false));
        assert!(keys.permits("network-key", false));
        assert_eq!(keys.id("local-key", true), Some("local"));
        assert!(keys.permits("anonymous-key", true));
        assert_eq!(keys.id("anonymous-key", true), None);
        assert_eq!(keys.entries().next().unwrap().name, Some("A \"label\""));
        let restored = parse(&keys.to_json().unwrap()).unwrap();
        assert!(restored.permits("network-key", false));
        assert_eq!(restored.entries().count(), 3);
    }

    #[test]
    fn rotation_and_revocation_are_transactional_and_do_not_log_secrets() {
        let mut keys = CallerKeys::new();
        keys.insert(ApiKey::new("one", "first-secret").unwrap())
            .unwrap();
        keys.insert(ApiKey::new("two", "second-secret").unwrap())
            .unwrap();
        assert!(keys.rotate("one", "second-secret").is_err());
        assert!(keys.permits("first-secret", true));
        keys.rotate("one", "replacement-secret").unwrap();
        assert!(!keys.permits("first-secret", true));
        assert!(keys.permits("replacement-secret", true));
        assert!(keys.revoke("one"));
        assert!(!keys.permits("replacement-secret", true));
        assert!(!keys.revoke("one"));
        assert_eq!(keys.entries().next().unwrap().id, Some("two"));
    }

    #[test]
    fn key_authentication_keeps_binding_under_loopback_trust() {
        let path = std::env::temp_dir().to_string_lossy().into_owned();
        let mut keys = CallerKeys::new();
        keys.insert(
            ApiKey::new("work", "actual-secret")
                .unwrap()
                .with_workspace(&path)
                .unwrap(),
        )
        .unwrap();
        let policy = AuthPolicy {
            keys: Some(&keys),
            token: Some("shared"),
            trust_loopback: true,
        };
        let caller = authenticate(
            &request("authorization", "Bearer actual-secret"),
            &policy,
            true,
        )
        .unwrap();
        assert_eq!(caller.key_id, Some("work"));
        assert_eq!(caller.workspace, Some(path.as_str()));
        assert_eq!(caller.authentication, Authentication::ApiKey);
        let trusted = authenticate(&request("x-api-key", "invalid"), &policy, true).unwrap();
        assert_eq!(trusted.workspace, None);
        assert_eq!(trusted.authentication, Authentication::TrustedLoopback);
        assert!(authenticate(&request("x-api-key", "actual-secret"), &policy, false).is_none());
        assert_eq!(
            authenticate(&request("x-goog-api-key", "shared"), &policy, false)
                .unwrap()
                .authentication,
            Authentication::SharedToken
        );
    }

    #[test]
    fn malformed_and_ambiguous_keys_fail_closed() {
        for source in [
            r#"{}"#,
            r#"[{"secret":""}]"#,
            r#"[{"secret":"x","lan":"yes"}]"#,
            r#"[{"secret":"x","unknown":true}]"#,
            r#"[{"secret":"x","secret":"y"}]"#,
            r#"[{"id":"same","secret":"x"},{"id":"same","secret":"y"}]"#,
            r#"[{"id":"a","secret":"same"},{"id":"b","secret":"same"}]"#,
            r#"[{"secret":"x","workspace":"/private"}]"#,
        ] {
            assert!(parse(source.as_bytes()).is_err(), "{source}");
        }
        let empty = parse(b"[]").unwrap();
        assert!(!empty.permits("", true));
        assert!(!empty.allows_lan());
        let policy = AuthPolicy {
            keys: Some(&empty),
            token: None,
            trust_loopback: false,
        };
        assert!(authenticate(&request("x-api-key", "any"), &policy, true).is_none());
    }
}
