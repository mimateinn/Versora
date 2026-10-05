//! Keep local detection, login, attempt eligibility and a real transport test distinct.
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use versora_engine::providers::ProviderStatus;

const TRANSPORT_PROOF_TTL: Duration = Duration::from_secs(300);

/// Only the explicit successful transport command creates this proof. Configuration
/// changes remove it; expiry and a newer unavailable probe prevent a connected label.
pub fn apply_probe(row: &mut Value, status: &ProviderStatus, proof: Option<Instant>) {
    row["configured"] = json!(status.usable);
    row["status"] = json!(if status.usable {
        "available"
    } else if status.signed_in == Some(false) {
        "signed-out"
    } else {
        "unavailable"
    });
    row["detail"] = json!(status.detail);
    row["models"] = json!(status.models);
    row["limitations"] = json!(status.limitations);
    row["probePerformed"] = json!(true);
    row["nativeDetected"] = if status.id.ends_with("_cli") {
        json!(status.present)
    } else {
        Value::Null
    };
    row["signedIn"] = json!(status.signed_in);
    row["availableForAttempt"] = json!(status.usable);
    row["transportVerified"] = json!(
        status.usable
            && proof.is_some_and(|when| when.elapsed() < TRANSPORT_PROOF_TTL)
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in_status() -> ProviderStatus {
        ProviderStatus {
            id: "codex_cli".into(),
            present: true,
            version: "0.159.0".into(),
            version_ok: true,
            signed_in: Some(true),
            usable: true,
            models: vec![],
            detail: "CLI reports signed in".into(),
            limitations: vec!["Full no-tool isolation is not certified".into()],
        }
    }

    #[test]
    fn local_login_does_not_claim_a_transport_connection() {
        let mut row = json!({"id":"codex_cli"});
        apply_probe(&mut row, &signed_in_status(), None);
        assert_eq!(row["probePerformed"], true);
        assert_eq!(row["nativeDetected"], true);
        assert_eq!(row["signedIn"], true);
        assert_eq!(row["availableForAttempt"], true);
        assert_eq!(row["transportVerified"], false);
        assert_eq!(row["limitations"][0], "Full no-tool isolation is not certified");
    }

    #[test]
    fn only_a_current_success_proof_can_mark_transport_verified() {
        let mut row = json!({"id":"codex_cli"});
        let status = signed_in_status();
        apply_probe(&mut row, &status, Some(Instant::now()));
        assert_eq!(row["transportVerified"], true);
        let expired = Instant::now().checked_sub(TRANSPORT_PROOF_TTL + Duration::from_secs(1)).unwrap();
        apply_probe(&mut row, &status, Some(expired));
        assert_eq!(row["transportVerified"], false);
        // A saved/deleted provider removes the proof instead of reusing a prior test.
        apply_probe(&mut row, &status, None);
        assert_eq!(row["transportVerified"], false);
    }

    #[test]
    fn a_new_signed_out_probe_invalidates_the_connected_display() {
        let mut row = json!({"id":"codex_cli"});
        let mut status = signed_in_status();
        status.signed_in = Some(false);
        status.usable = false;
        apply_probe(&mut row, &status, Some(Instant::now()));
        assert_eq!(row["signedIn"], false);
        assert_eq!(row["status"], "signed-out");
        assert_eq!(row["availableForAttempt"], false);
        assert_eq!(row["transportVerified"], false);
    }

    #[test]
    fn an_api_key_probe_never_claims_a_native_cli_or_connection() {
        let mut row = json!({"id":"openai"});
        let mut status = signed_in_status();
        status.id = "openai".into();
        status.signed_in = None;
        status.version_ok = false;
        apply_probe(&mut row, &status, None);
        assert!(row["nativeDetected"].is_null());
        assert!(row["signedIn"].is_null());
        assert_eq!(row["transportVerified"], false);
    }
}
