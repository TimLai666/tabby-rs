//! Regression tests for SSH credential loading before authentication.
use super::*;
use crate::security::keychain::{CredentialError, CredentialStore};
use secrecy::{ExposeSecret, SecretString};
use std::collections::BTreeMap;
use std::future::{ready, Ready};
use std::sync::{Arc, Mutex};

fn request_with(auth: Vec<AuthMethodRef>, enabled: bool) -> SshConnectRequest {
    SshConnectRequest {
        profile_id: "ssh:test".into(),
        connection_id: Some("connection:test".into()),
        host: "example.test".into(),
        port: 22,
        username: Some("alice".into()),
        auth,
        terminal: TerminalRequest {
            term: "xterm-256color".into(),
            columns: 80,
            rows: 24,
            pixel_width: None,
            pixel_height: None,
        },
        keepalive: None,
        environment: BTreeMap::new(),
        x11: false,
        x11_display: None,
        agent_forward: false,
        agent_forwarding: None,
        jump_chain: Vec::new(),
        load_stored_password: enabled,
    }
}

fn response(value: &str) -> Ready<Result<Vec<String>, SshError>> {
    ready(Ok(vec![value.to_owned()]))
}

fn prompt_values(prompt: &SshAuthPrompt) {
    assert_eq!(prompt.name, "");
    assert_eq!(prompt.instructions, "");
    assert!(prompt.prompts.is_empty());
    assert!(prompt.password.is_none());
    assert_eq!(
        prompt.stored_password.as_ref().unwrap().host,
        "example.test"
    );
    assert_eq!(prompt.stored_password.as_ref().unwrap().port, 22);
    assert_eq!(prompt.stored_password.as_ref().unwrap().username, "alice");
}

#[tokio::test]
async fn disabled_flag_skips_load_and_enabled_flag_loads_for_every_auth_mode() {
    let legacy: SshConnectRequest = serde_json::from_value(serde_json::json!({
        "profileId": "ssh:legacy", "host": "example.test", "port": 22,
        "username": "alice", "auth": [], "terminal": {
            "term": "xterm-256color", "columns": 80, "rows": 24,
            "pixelWidth": null, "pixelHeight": null
        }, "keepalive": null
    }))
    .unwrap();
    assert!(
        !legacy.load_stored_password,
        "requests that omit the opt-in flag retain legacy behavior"
    );
    let modes = [
        serde_json::json!([]),
        serde_json::json!([{ "type": "password", "secretRef": "ssh-password://vault" }]),
        serde_json::json!([{ "type": "keyboardInteractive", "secretRef": "ssh-password://vault" }]),
        serde_json::json!([{ "type": "privateKey", "fileRef": "key:test", "passphraseRef": null }]),
        serde_json::json!([{ "type": "agent", "socket": null }]),
    ];
    for methods in modes {
        let auth: Vec<AuthMethodRef> = serde_json::from_value(methods).unwrap();
        let disabled = request_with(auth.clone(), false);
        let calls = Arc::new(Mutex::new(0));
        let count = calls.clone();
        let prepared = super::prepare_stored_password(&disabled, "alice", move |_| {
            *count.lock().unwrap() += 1;
            response("never")
        })
        .await
        .unwrap();
        assert!(prepared.is_none());
        assert_eq!(*calls.lock().unwrap(), 0);

        let enabled = request_with(auth, true);
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let captured = prompts.clone();
        let prepared = super::prepare_stored_password(&enabled, "alice", move |prompt| {
            prompt_values(&prompt);
            captured.lock().unwrap().push(prompt);
            response("loaded-secret")
        })
        .await
        .unwrap();
        assert_eq!(
            prepared
                .as_ref()
                .map(|value| value.as_ref().map(|s| s.expose_secret().as_str())),
            Some(Some("loaded-secret"))
        );
        assert_eq!(prompts.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn resolved_account_and_empty_or_null_response_have_one_empty_candidate() {
    let mut target = request_with(vec![], true);
    target.username = Some("$SSH_USER".into());
    target.host = "resolved.test".into();
    target.port = 2202;
    let prepared = super::prepare_stored_password(&target, "resolved-user", |prompt| {
        assert_eq!(
            prompt.stored_password.as_ref().unwrap().host,
            "resolved.test"
        );
        assert_eq!(prompt.stored_password.as_ref().unwrap().port, 2202);
        assert_eq!(
            prompt.stored_password.as_ref().unwrap().username,
            "resolved-user"
        );
        response("")
    })
    .await
    .unwrap();
    assert!(matches!(prepared, Some(None)));

    let absent = super::prepare_stored_password(&target, "resolved-user", |_| {
        ready(Ok(vec![String::new()]))
    })
    .await
    .unwrap();
    assert!(matches!(absent, Some(None)));
    let rejected = super::prepare_stored_password(&target, "resolved-user", |_| {
        ready(Err(SshError::AuthenticationRejected))
    })
    .await;
    assert!(rejected.is_err());
}

#[tokio::test]
async fn malformed_reply_length_and_native_error_fail_without_password_candidates() {
    let request = request_with(vec![], true);
    for result in [
        Ok(Vec::new()),
        Ok(vec!["one".into(), "two".into()]),
        Err(SshError::Connection),
    ] {
        let outcome =
            super::prepare_stored_password(&request, "alice", move |_| ready(result)).await;
        assert!(outcome.is_err());
    }
}

#[test]
fn prepared_snapshot_resolves_vault_and_keychain_without_another_storage_read() {
    let credentials = CredentialState::default();
    let locked = SecretState::default();
    let methods: Vec<AuthMethodRef> = serde_json::from_value(serde_json::json!([
        { "type": "providedPassword", "password": "configured" },
        { "type": "password", "secretRef": "ssh-password://vault" },
        { "type": "keyboardInteractive", "secretRef": "ssh-password://keychain" },
        { "type": "promptPassword" }
    ]))
    .unwrap();
    let request = request_with(methods.clone(), true);
    // SecretState stays locked and empty. The prepared value is supplied by the
    // renderer's TauriPasswordStorageService, which selects `ssh:password`; native
    // legacy vault lookup uses another selector and must not run for this path.
    let snapshot = Some(Some(secrecy::SecretString::new("saved".into())));
    let prepared = super::resolve_auth_passwords(
        &methods,
        &request,
        "alice",
        &locked,
        &credentials,
        snapshot.as_ref(),
    )
    .unwrap();
    let values: Vec<_> = prepared
        .values
        .iter()
        .map(|value| value.as_ref().map(|secret| secret.expose_secret().as_str()))
        .collect();
    assert_eq!(
        values,
        vec![Some("configured"), Some("saved"), Some("saved"), None]
    );
    assert!(!format!("{prepared:?}").contains("saved"));

    let legacy =
        super::resolve_auth_passwords(&methods, &request, "alice", &locked, &credentials, None)
            .unwrap();
    assert!(legacy.values[1].is_none());
}

#[tokio::test]
async fn target_and_jump_hosts_prepare_independent_identity_bound_snapshots() {
    let methods: Vec<AuthMethodRef> = serde_json::from_value(serde_json::json!([
        { "type": "password", "secretRef": "ssh-password://vault" }
    ]))
    .unwrap();
    let credentials = CredentialState::default();
    let locked = SecretState::default();
    let mut target = request_with(methods.clone(), true);
    target.host = "target.test".into();
    target.port = 22;
    let mut hop = request_with(methods.clone(), true);
    hop.host = "hop.test".into();
    hop.port = 2202;
    hop.username = Some("jump-default".into());

    let target_snapshot = super::prepare_stored_password(&target, "resolved-target", |prompt| {
        let identity = prompt.stored_password.unwrap();
        assert_eq!(
            (
                identity.host.as_str(),
                identity.port,
                identity.username.as_str()
            ),
            ("target.test", 22, "resolved-target")
        );
        response("target-secret")
    })
    .await
    .unwrap();
    let hop_snapshot = super::prepare_stored_password(&hop, "resolved-hop", |prompt| {
        let identity = prompt.stored_password.unwrap();
        assert_eq!(
            (
                identity.host.as_str(),
                identity.port,
                identity.username.as_str()
            ),
            ("hop.test", 2202, "resolved-hop")
        );
        response("hop-secret")
    })
    .await
    .unwrap();
    let target_values = super::resolve_auth_passwords(
        &methods,
        &target,
        "resolved-target",
        &locked,
        &credentials,
        target_snapshot.as_ref(),
    )
    .unwrap();
    let hop_values = super::resolve_auth_passwords(
        &methods,
        &hop,
        "resolved-hop",
        &locked,
        &credentials,
        hop_snapshot.as_ref(),
    )
    .unwrap();
    assert_eq!(
        target_values.values[0].as_ref().unwrap().expose_secret(),
        "target-secret"
    );
    assert_eq!(
        hop_values.values[0].as_ref().unwrap().expose_secret(),
        "hop-secret"
    );
}

#[test]
fn configured_candidates_keep_order_and_duplicate_handling_with_snapshot() {
    let credentials = CredentialState::default();
    let locked = SecretState::default();
    let methods: Vec<AuthMethodRef> = serde_json::from_value(serde_json::json!([
        { "type": "providedPassword", "password": "configured" },
        { "type": "keyboardInteractive", "password": "configured" },
        { "type": "keyboardInteractive", "secretRef": "ssh-password://vault" },
        { "type": "password", "secretRef": "ssh-password://vault" },
        { "type": "promptPassword" }
    ]))
    .unwrap();
    let request = request_with(methods.clone(), true);
    let snapshot = Some(Some(secrecy::SecretString::new("configured".into())));
    let resolved = super::resolve_auth_passwords(
        &methods,
        &request,
        "alice",
        &locked,
        &credentials,
        snapshot.as_ref(),
    )
    .unwrap();
    let values: Vec<_> = resolved
        .values
        .iter()
        .map(|item| item.as_ref().map(|secret| secret.expose_secret().as_str()))
        .collect();
    assert_eq!(
        values,
        vec![
            Some("configured"),
            Some("configured"),
            None,
            Some("configured"),
            None
        ]
    );
    let secret = secrecy::SecretString::new("must-not-leak".into());
    assert!(!format!("{secret:?}").contains("must-not-leak"));
}

#[test]
fn explicit_secret_references_keep_the_existing_lookup_path() {
    struct FixtureStore;
    impl CredentialStore for FixtureStore {
        fn get(
            &self,
            _: CredentialNamespace,
            address: &CredentialAddress,
        ) -> Result<Option<secrecy::SecretString>, CredentialError> {
            Ok(
                (address.service == "ssh@example.test:22" && address.account == "alice")
                    .then(|| secrecy::SecretString::new("explicit-reference-secret".into())),
            )
        }
        fn put(
            &self,
            _: CredentialNamespace,
            _: &CredentialAddress,
            _: &str,
        ) -> Result<(), CredentialError> {
            Ok(())
        }
        fn delete(
            &self,
            _: CredentialNamespace,
            _: &CredentialAddress,
        ) -> Result<bool, CredentialError> {
            Ok(false)
        }
    }
    let credentials = CredentialState::with_store(Arc::new(FixtureStore));
    let locked = SecretState::default();
    let methods: Vec<AuthMethodRef> = serde_json::from_value(serde_json::json!([
        { "type": "password", "secretRef": "keychain://ssh@example.test:22/alice" }
    ]))
    .unwrap();
    let request = request_with(methods.clone(), true);
    let renderer_snapshot = Some(Some(secrecy::SecretString::new(
        "renderer-stored-password".into(),
    )));
    let resolved = super::resolve_auth_passwords(
        &methods,
        &request,
        "alice",
        &locked,
        &credentials,
        renderer_snapshot.as_ref(),
    )
    .unwrap();
    assert_eq!(
        resolved.values[0].as_ref().unwrap().expose_secret(),
        "explicit-reference-secret"
    );
}

#[tokio::test]
async fn manager_stops_before_any_auth_method_when_renderer_load_cannot_be_started() {
    struct SpyContext {
        calls: usize,
    }
    #[async_trait::async_trait]
    impl SshAuthContext for SpyContext {
        async fn authenticate_none(&mut self, _: &str) -> Result<bool, SshError> {
            self.calls += 1;
            panic!("auth started before stored-password preparation")
        }
        async fn authenticate_password(
            &mut self,
            _: &str,
            _: &SecretString,
        ) -> Result<bool, SshError> {
            self.calls += 1;
            panic!("password auth started before stored-password preparation")
        }
        async fn authenticate_private_key(
            &mut self,
            _: &str,
            _: PrivateKeyMaterial,
        ) -> Result<bool, SshError> {
            self.calls += 1;
            panic!("key auth started before stored-password preparation")
        }
        async fn authenticate_agent(&mut self, _: &str, _: Option<&str>) -> Result<bool, SshError> {
            self.calls += 1;
            panic!("agent auth started before stored-password preparation")
        }
        async fn authenticate_keyboard_interactive_start(
            &mut self,
            _: &str,
        ) -> Result<KeyboardInteractiveResponse, SshError> {
            self.calls += 1;
            panic!("keyboard auth started before stored-password preparation")
        }
        async fn authenticate_keyboard_interactive_respond(
            &mut self,
            _: Vec<String>,
        ) -> Result<KeyboardInteractiveResponse, SshError> {
            self.calls += 1;
            panic!("keyboard response started before stored-password preparation")
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let secrets = SecretState::default();
    let credentials = CredentialState::default();
    let auth = serde_json::from_value(serde_json::json!([
        { "type": "agent", "socket": null },
        { "type": "password", "secretRef": "ssh-password://vault" },
        { "type": "keyboardInteractive", "secretRef": "ssh-password://vault" },
        { "type": "privateKey", "fileRef": "key:test", "passphraseRef": null },
        { "type": "promptPassword" }
    ]))
    .unwrap();
    let authenticator = ManagerAuthenticator {
        manager: SshManager::new(directory.path().to_path_buf()),
        app: None,
        request: request_with(auth, true),
        secrets: &secrets,
        credentials: &credentials,
        used_private_key: Mutex::new(false),
        resolved_username: Mutex::new(Some("resolved-alice".into())),
    };
    let mut context = SpyContext { calls: 0 };
    let result = authenticator
        .authenticate(&mut context, "ignored", &[])
        .await;
    assert!(matches!(result, Err(SshError::Closed)));
    assert_eq!(context.calls, 0);
}

#[test]
fn jump_request_keeps_each_hops_loading_flag_and_legacy_defaults() {
    let legacy: SshJumpRequest = serde_json::from_value(serde_json::json!({
        "host": "hop.test", "port": 2202, "username": null, "auth": []
    }))
    .unwrap();
    assert!(!legacy.load_stored_password);
    for root_enabled in [false, true] {
        for hop_enabled in [false, true] {
            let root = request_with(vec![], root_enabled);
            let mut hop = legacy.clone();
            hop.load_stored_password = hop_enabled;
            let request = super::jump_request(&root, &hop, "shared-connection", 1);
            assert_eq!(request.load_stored_password, hop_enabled);
            assert_eq!(request.connection_id.as_deref(), Some("shared-connection"));
            assert_eq!((request.host.as_str(), request.port), ("hop.test", 2202));
        }
    }
}
