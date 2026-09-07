//! Tests for the sign-in and refresh flow. Every request goes to a wiremock server and
//! every wait goes to a recording `sleep`, so no test touches the network or the clock.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::*;
use crate::auth::msa::MsaEndpoints;
use crate::auth::secrets::{MemoryStore, SecretStoreKind};
use crate::events::null_sink;
use crate::http::HttpClient;
use crate::paths::Root;

const CLIENT_ID: &str = "00000000-0000-0000-0000-000000000abc";
const PROFILE_ID: &str = "b50ad385829d3141a2167e7d7539ba7f";
const ACCOUNT_ID: &str = "b50ad385-829d-3141-a216-7e7d7539ba7f";

/// Answers a sequence of bodies, one per call, repeating the last one after that.
struct Sequence {
    answers: Vec<(u16, String)>,
    calls: AtomicUsize,
}

impl Sequence {
    fn new(answers: &[(u16, &str)]) -> Self {
        Sequence {
            answers: answers
                .iter()
                .map(|(status, body)| (*status, (*body).to_string()))
                .collect(),
            calls: AtomicUsize::new(0),
        }
    }
}

impl Respond for Sequence {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let (status, body) = &self.answers[n.min(self.answers.len() - 1)];
        ResponseTemplate::new(*status).set_body_string(body.clone())
    }
}

fn client() -> HttpClient {
    HttpClient::new()
        .expect("client builds")
        .with_backoff(vec![Duration::ZERO; 3])
}

fn endpoints(base: &str) -> MsaEndpoints {
    MsaEndpoints {
        device_code: format!("{base}/devicecode"),
        token: format!("{base}/token"),
        xbl: format!("{base}/xbl"),
        xsts: format!("{base}/xsts"),
        mc_login: format!("{base}/mclogin"),
        profile: format!("{base}/profile"),
    }
}

fn msa(server: &MockServer) -> Msa {
    Msa::new(client(), endpoints(&server.uri()), CLIENT_ID.to_string())
}

fn accounts() -> (tempfile::TempDir, Accounts) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = Root::from_path(dir.path());
    let accounts = Accounts::new(&root);
    (dir, accounts)
}

/// Mounts the device-code endpoint with a five second interval and a `expires_in` lifetime.
async fn mount_device_code(server: &MockServer, interval: u64, expires_in: u64) {
    Mock::given(method("POST"))
        .and(path("/devicecode"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"user_code":"ABCD-EFGH","device_code":"dev-secret",
                 "verification_uri":"https://microsoft.com/link",
                 "expires_in":{expires_in},"interval":{interval},"message":"Sign in."}}"#
        )))
        .mount(server)
        .await;
}

/// Mounts Xbox Live, XSTS, the Minecraft login, and the profile read with fixed answers.
async fn mount_chain(server: &MockServer, mc_token: &str) {
    Mock::given(method("POST"))
        .and(path("/xbl"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"Token":"xbl-token","DisplayClaims":{"xui":[{"uhs":"user-hash"}]}}"#,
        ))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/xsts"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"Token":"xsts-token","DisplayClaims":{"xui":[{"uhs":"user-hash","xid":"2535"}]}}"#,
        ))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/mclogin"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            r#"{{"access_token":"{mc_token}","expires_in":86400}}"#
        )))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(format!(r#"{{"id":"{PROFILE_ID}","name":"Notch"}}"#)),
        )
        .mount(server)
        .await;
}

/// A `sleep` that records what it was asked to wait for and returns at once.
fn recording_sleep() -> (
    Arc<Mutex<Vec<Duration>>>,
    impl Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync,
) {
    let waits = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&waits);
    let sleep = move |d: Duration| {
        recorder.lock().expect("lock").push(d);
        Box::pin(std::future::ready(())) as BoxFuture<'static, ()>
    };
    (waits, sleep)
}

fn msa_account(expires: Option<&str>) -> Account {
    Account {
        id: ACCOUNT_ID.to_string(),
        name: "Notch".to_string(),
        kind: AccountKind::Msa,
        mc_token: Some("old-mc-token".to_string()),
        mc_token_expires: expires.map(str::to_string),
        xuid: Some("2535".to_string()),
        refresh_store: Some(SecretStoreKind::Memory),
    }
}

fn rfc3339(at: OffsetDateTime) -> String {
    at.format(&Rfc3339).expect("formats")
}

const PENDING: (u16, &str) = (400, r#"{"error":"authorization_pending"}"#);
const SLOW_DOWN: (u16, &str) = (400, r#"{"error":"slow_down"}"#);
const READY: (u16, &str) = (
    200,
    r#"{"access_token":"msa-access","refresh_token":"refresh-1"}"#,
);

#[tokio::test]
async fn login_polls_until_ready_then_stores_the_account_and_the_refresh_token() {
    let server = MockServer::start().await;
    mount_device_code(&server, 5, 900).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(Sequence::new(&[PENDING, PENDING, READY]))
        .mount(&server)
        .await;
    mount_chain(&server, "mc-token").await;

    let msa = msa(&server);
    let secrets = MemoryStore::new();
    let (_dir, store) = accounts();
    let sink = null_sink();
    let ctx = LoginCtx {
        msa: &msa,
        secrets: &secrets,
        accounts: &store,
        sink: &sink,
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let codes = Arc::clone(&seen);
    let on_code = move |code: &DeviceCode| codes.lock().expect("lock").push(code.user_code.clone());
    let (waits, sleep) = recording_sleep();

    let account = login_device_code(&ctx, &on_code, &sleep)
        .await
        .expect("login finishes");

    assert_eq!(account.id, ACCOUNT_ID);
    assert_eq!(account.name, "Notch");
    assert_eq!(account.kind, AccountKind::Msa);
    assert_eq!(account.mc_token.as_deref(), Some("mc-token"));
    assert_eq!(account.xuid.as_deref(), Some("2535"));
    assert_eq!(account.refresh_store, Some(SecretStoreKind::Memory));
    assert!(account.mc_token_expires.is_some());

    assert_eq!(*seen.lock().expect("lock"), vec!["ABCD-EFGH".to_string()]);
    assert_eq!(
        *waits.lock().expect("lock"),
        vec![Duration::from_secs(5); 3]
    );
    assert_eq!(store.active().expect("active"), Some(account.clone()));
    assert_eq!(
        secrets.get(ACCOUNT_ID).expect("get"),
        Some("refresh-1".to_string())
    );
}

#[tokio::test]
async fn slow_down_adds_five_seconds_to_the_poll_interval() {
    let server = MockServer::start().await;
    mount_device_code(&server, 5, 900).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(Sequence::new(&[SLOW_DOWN, PENDING, READY]))
        .mount(&server)
        .await;
    mount_chain(&server, "mc-token").await;

    let msa = msa(&server);
    let secrets = MemoryStore::new();
    let (_dir, store) = accounts();
    let sink = null_sink();
    let ctx = LoginCtx {
        msa: &msa,
        secrets: &secrets,
        accounts: &store,
        sink: &sink,
    };
    let (waits, sleep) = recording_sleep();

    login_device_code(&ctx, &|_| {}, &sleep)
        .await
        .expect("login finishes");

    assert_eq!(
        *waits.lock().expect("lock"),
        vec![
            Duration::from_secs(5),
            Duration::from_secs(10),
            Duration::from_secs(10),
        ]
    );
}

#[tokio::test]
async fn login_gives_up_when_the_sleeps_reach_the_code_lifetime() {
    let server = MockServer::start().await;
    mount_device_code(&server, 5, 10).await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(Sequence::new(&[PENDING]))
        .mount(&server)
        .await;

    let msa = msa(&server);
    let secrets = MemoryStore::new();
    let (_dir, store) = accounts();
    let sink = null_sink();
    let ctx = LoginCtx {
        msa: &msa,
        secrets: &secrets,
        accounts: &store,
        sink: &sink,
    };
    let (waits, sleep) = recording_sleep();

    let err = login_device_code(&ctx, &|_| {}, &sleep)
        .await
        .expect_err("an unapproved code expires");

    assert!(matches!(err, Error::DeviceCodeExpired), "got {err:?}");
    assert_eq!(waits.lock().expect("lock").len(), 2);
    assert!(store.list().expect("list").is_empty());
}

#[tokio::test]
async fn refresh_rotates_the_stored_token_and_updates_the_minecraft_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"access_token":"msa-access-2","refresh_token":"refresh-2"}"#),
        )
        .mount(&server)
        .await;
    mount_chain(&server, "mc-token-2").await;

    let msa = msa(&server);
    let secrets = MemoryStore::new();
    secrets.put(ACCOUNT_ID, "refresh-1").expect("put");
    let (_dir, store) = accounts();
    let stored = msa_account(Some("2020-01-01T00:00:00Z"));
    store.add(stored.clone()).expect("add");
    let sink = null_sink();
    let ctx = LoginCtx {
        msa: &msa,
        secrets: &secrets,
        accounts: &store,
        sink: &sink,
    };

    let fresh = refresh_account(&ctx, &stored).await.expect("refresh works");

    assert_eq!(fresh.id, ACCOUNT_ID);
    assert_eq!(fresh.mc_token.as_deref(), Some("mc-token-2"));
    assert_ne!(fresh.mc_token_expires, stored.mc_token_expires);
    assert_eq!(
        secrets.get(ACCOUNT_ID).expect("get"),
        Some("refresh-2".to_string())
    );
    assert_eq!(store.list().expect("list"), vec![fresh]);
}

#[tokio::test]
async fn refresh_without_a_stored_token_reports_no_refresh_token() {
    let server = MockServer::start().await;
    let msa = msa(&server);
    let secrets = MemoryStore::new();
    let (_dir, store) = accounts();
    let sink = null_sink();
    let ctx = LoginCtx {
        msa: &msa,
        secrets: &secrets,
        accounts: &store,
        sink: &sink,
    };

    let err = refresh_account(&ctx, &msa_account(None))
        .await
        .expect_err("there is nothing to refresh with");

    assert!(matches!(err, Error::NoRefreshToken), "got {err:?}");
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "no request is made without a token"
    );
}

#[tokio::test]
async fn refresh_of_a_profile_for_another_account_is_a_mismatch() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"access_token":"msa-access-2","refresh_token":"refresh-2"}"#),
        )
        .mount(&server)
        .await;
    mount_chain(&server, "mc-token-2").await;

    let msa = msa(&server);
    let secrets = MemoryStore::new();
    secrets.put("someone-else", "refresh-1").expect("put");
    let (_dir, store) = accounts();
    let sink = null_sink();
    let ctx = LoginCtx {
        msa: &msa,
        secrets: &secrets,
        accounts: &store,
        sink: &sink,
    };
    let other = Account {
        id: "someone-else".to_string(),
        ..msa_account(None)
    };

    let err = refresh_account(&ctx, &other)
        .await
        .expect_err("the profile belongs to another account");

    assert!(matches!(err, Error::AccountMismatch { .. }), "got {err:?}");
    assert_eq!(
        secrets.get("someone-else").expect("get"),
        Some("refresh-1".to_string()),
        "a mismatch leaves the stored token alone"
    );
}

#[tokio::test]
async fn ensure_fresh_leaves_a_token_with_time_left_alone() {
    let server = MockServer::start().await;
    let msa = msa(&server);
    let secrets = MemoryStore::new();
    let (_dir, store) = accounts();
    let sink = null_sink();
    let ctx = LoginCtx {
        msa: &msa,
        secrets: &secrets,
        accounts: &store,
        sink: &sink,
    };
    let now = OffsetDateTime::now_utc();
    let account = msa_account(Some(&rfc3339(now + Duration::from_secs(3600))));

    let same = ensure_fresh(&ctx, account.clone(), now)
        .await
        .expect("a fresh token needs nothing");

    assert_eq!(same, account);
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "a fresh token makes no request"
    );
}

#[tokio::test]
async fn ensure_fresh_refreshes_a_token_inside_the_margin() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"access_token":"msa-access-2","refresh_token":"refresh-2"}"#),
        )
        .mount(&server)
        .await;
    mount_chain(&server, "mc-token-2").await;

    let msa = msa(&server);
    let secrets = MemoryStore::new();
    secrets.put(ACCOUNT_ID, "refresh-1").expect("put");
    let (_dir, store) = accounts();
    let sink = null_sink();
    let ctx = LoginCtx {
        msa: &msa,
        secrets: &secrets,
        accounts: &store,
        sink: &sink,
    };
    let now = OffsetDateTime::now_utc();
    let account = msa_account(Some(&rfc3339(now + Duration::from_secs(60))));

    let fresh = ensure_fresh(&ctx, account, now)
        .await
        .expect("an expiring token refreshes");

    assert_eq!(fresh.mc_token.as_deref(), Some("mc-token-2"));
}

#[tokio::test]
async fn ensure_fresh_passes_an_offline_account_through() {
    let server = MockServer::start().await;
    let msa = msa(&server);
    let secrets = MemoryStore::new();
    let (_dir, store) = accounts();
    let sink = null_sink();
    let ctx = LoginCtx {
        msa: &msa,
        secrets: &secrets,
        accounts: &store,
        sink: &sink,
    };
    let account = crate::auth::offline::offline_account("Notch");

    let same = ensure_fresh(&ctx, account.clone(), OffsetDateTime::now_utc())
        .await
        .expect("an offline account never refreshes");

    assert_eq!(same, account);
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty(),
        "an offline account makes no request"
    );
}
