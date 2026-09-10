//! Secrets live in the Windows Credential Manager, never in the database.

use crate::error::{Result, SkimError};

const SERVICE: &str = "Skim";

/// `ERROR_NOT_ENOUGH_MEMORY`. Credential Manager answers a write with this when
/// the user's vault won't take another entry — its size cap is shared with every
/// other app's stored credentials — not when the machine is short on memory.
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;

/// The Win32 code behind a keyring failure, when there is one.
fn win32_code(e: &keyring::Error) -> Option<u32> {
    match e {
        keyring::Error::PlatformFailure(inner) | keyring::Error::NoStorageAccess(inner) => {
            inner.downcast_ref::<keyring::windows::Error>().map(|w| w.0)
        }
        _ => None,
    }
}

/// Give the UI a stable code to explain the failure by, and keep the raw Windows
/// text as the detail — it is what a bug report needs.
fn store_err(e: keyring::Error) -> SkimError {
    let code = if win32_code(&e) == Some(ERROR_NOT_ENOUGH_MEMORY) {
        "secrets_full"
    } else if matches!(e, keyring::Error::NoStorageAccess(_)) {
        "secrets_unavailable"
    } else {
        "secrets"
    };
    tracing::error!(error = %e, code, "credential store failed");
    SkimError::other(code, format!("credential store: {e}"))
}

fn entry(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, account).map_err(store_err)
}

pub fn set(account: &str, secret: &str) -> Result<()> {
    entry(account)?.set_password(secret).map_err(store_err)
}

pub fn get(account: &str) -> Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(store_err(e)),
    }
}

pub fn delete(account: &str) -> Result<()> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(store_err(e)),
    }
}

/// Key under which the mail credential for an account is stored. Holds the
/// password for `auth_kind = 'password'` or the OAuth refresh token for
/// `auth_kind = 'oauth'`.
pub fn mail_key(account_id: &str) -> String {
    format!("mail:{account_id}")
}

pub const ANTHROPIC_KEY: &str = "anthropic_api_key";
pub const OPENROUTER_KEY: &str = "openrouter_api_key";
/// Optional key for the user-supplied OpenAI-compatible endpoint.
pub const CUSTOM_KEY: &str = "custom_api_key";

#[cfg(test)]
mod tests {
    use super::*;

    fn platform_failure(code: u32) -> keyring::Error {
        keyring::Error::PlatformFailure(Box::new(keyring::windows::Error(code)))
    }

    #[test]
    fn a_full_vault_is_reported_as_such() {
        let err = store_err(platform_failure(ERROR_NOT_ENOUGH_MEMORY));
        assert_eq!(err.code(), "secrets_full");
        // The Windows code stays in the message, for bug reports.
        assert!(err.to_string().contains("Windows error code 8"));
    }

    #[test]
    fn other_failures_keep_their_own_codes() {
        assert_eq!(store_err(platform_failure(1)).code(), "secrets");
        assert_eq!(
            store_err(keyring::Error::NoStorageAccess(Box::new(
                keyring::windows::Error(1312)
            )))
            .code(),
            "secrets_unavailable"
        );
    }
}
