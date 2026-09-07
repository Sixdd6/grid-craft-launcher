//! Tests for the Microsoft login chain. Every request goes to a wiremock server: the
//! production endpoints are never reached.

use super::*;
use std::time::Duration;
use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const CLIENT_ID: &str = "00000000-0000-0000-0000-000000000abc";

fn client() -> HttpClient {
    HttpClient::new()
        .expect("client builds")
        .with_backoff(vec![Duration::ZERO; 3])
}

/// Endpoints that all point at one mock server, each on its own path.
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

fn a_device_code() -> DeviceCode {
    DeviceCode {
        user_code: "ABCD-EFGH".to_string(),
        verification_uri: "https://microsoft.com/link".to_string(),
        message: "go here".to_string(),
        interval_secs: 5,
        expires_in_secs: 900,
        device_code: "dev-secret".to_string(),
    }
}

fn an_xbl_token() -> XblToken {
    XblToken {
        token: "xbl-token".to_string(),
        uhs: "user-hash".to_string(),
    }
}

fn an_xsts_token() -> XstsToken {
    XstsToken {
        token: "xsts-token".to_string(),
        uhs: "user-hash".to_string(),
        xuid: Some("2535".to_string()),
    }
}

/// Mounts one 400 answer on the token endpoint and polls once.
async fn poll_with_oauth_error(code: &str) -> Result<Poll, Error> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string(format!(
            r#"{{"error":"{code}","error_description":"never read: dev-secret"}}"#
        )))
        .expect(1)
        .mount(&server)
        .await;
    msa(&server).poll_device_code_once(&a_device_code()).await
}

#[tokio::test]
async fn start_device_code_sends_the_client_id_and_scope_and_parses_the_answer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/devicecode"))
        .and(header("content-type", "application/x-www-form-urlencoded"))
        .and(body_string_contains(format!("client_id={CLIENT_ID}")))
        .and(body_string_contains(
            "scope=XboxLive.signin%20offline_access",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"user_code":"ABCD-EFGH","device_code":"dev-secret",
                 "verification_uri":"https://microsoft.com/link",
                 "expires_in":900,"interval":5,"message":"Sign in at the link."}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let code = msa(&server)
        .start_device_code()
        .await
        .expect("device code starts");
    assert_eq!(code.user_code, "ABCD-EFGH");
    assert_eq!(code.device_code, "dev-secret");
    assert_eq!(code.verification_uri, "https://microsoft.com/link");
    assert_eq!(code.interval_secs, 5);
    assert_eq!(code.expires_in_secs, 900);
    assert_eq!(code.message, "Sign in at the link.");
}

#[tokio::test]
async fn poll_maps_authorization_pending_to_pending() {
    let poll = poll_with_oauth_error("authorization_pending")
        .await
        .expect("pending is not an error");
    assert!(matches!(poll, Poll::Pending), "got {poll:?}");
}

#[tokio::test]
async fn poll_maps_slow_down() {
    let poll = poll_with_oauth_error("slow_down")
        .await
        .expect("slow_down is not an error");
    assert!(matches!(poll, Poll::SlowDown), "got {poll:?}");
}

#[tokio::test]
async fn poll_maps_expired_token() {
    let err = poll_with_oauth_error("expired_token")
        .await
        .expect_err("an expired code fails");
    assert!(matches!(err, Error::DeviceCodeExpired), "got {err:?}");
}

#[tokio::test]
async fn poll_maps_authorization_declined() {
    let err = poll_with_oauth_error("authorization_declined")
        .await
        .expect_err("a declined sign-in fails");
    assert!(matches!(err, Error::DeviceCodeDeclined), "got {err:?}");
}

#[tokio::test]
async fn poll_reports_an_unknown_error_code_without_the_description() {
    let err = poll_with_oauth_error("invalid_client")
        .await
        .expect_err("an unknown code fails");
    match &err {
        Error::Oauth(code) => assert_eq!(code, "invalid_client"),
        other => panic!("got {other:?}"),
    }
    assert!(!err.to_string().contains("dev-secret"));
}

#[tokio::test]
async fn poll_sends_the_device_code_grant_and_returns_both_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains(
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code",
        ))
        .and(body_string_contains("device_code=dev-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_token":"msa-access","refresh_token":"msa-refresh","expires_in":3600}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let poll = msa(&server)
        .poll_device_code_once(&a_device_code())
        .await
        .expect("poll succeeds");
    match poll {
        Poll::Ready(tokens) => {
            assert_eq!(tokens.access_token, "msa-access");
            assert_eq!(tokens.refresh_token, "msa-refresh");
        }
        other => panic!("got {other:?}"),
    }
}

#[tokio::test]
async fn refresh_sends_the_refresh_grant() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=old-refresh"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let tokens = msa(&server)
        .refresh("old-refresh")
        .await
        .expect("refresh succeeds");
    assert_eq!(tokens.access_token, "new-access");
    assert_eq!(tokens.refresh_token, "new-refresh");
}

#[tokio::test]
async fn refresh_maps_an_oauth_failure_to_its_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":"invalid_grant","error_description":"token old-refresh expired"}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let err = msa(&server)
        .refresh("old-refresh")
        .await
        .expect_err("an expired refresh token fails");
    match &err {
        Error::Oauth(code) => assert_eq!(code, "invalid_grant"),
        other => panic!("got {other:?}"),
    }
    assert!(!err.to_string().contains("old-refresh"));
}

#[tokio::test]
async fn xbl_builds_the_documented_body_and_parses_the_user_hash() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/xbl"))
        .and(header("accept", "application/json"))
        .and(body_partial_json(serde_json::json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": "d=msa-access",
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"Token":"xbl-token","DisplayClaims":{"xui":[{"uhs":"user-hash"}]}}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let xbl = msa(&server).xbl("msa-access").await.expect("xbl succeeds");
    assert_eq!(xbl.token, "xbl-token");
    assert_eq!(xbl.uhs, "user-hash");
}

#[tokio::test]
async fn xbl_maps_a_failed_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/xbl"))
        .respond_with(ResponseTemplate::new(400))
        .expect(1)
        .mount(&server)
        .await;

    let err = msa(&server)
        .xbl("msa-access")
        .await
        .expect_err("a 400 fails");
    assert!(matches!(err, Error::Xbl(_)), "got {err:?}");
}

#[tokio::test]
async fn xsts_builds_the_documented_body_and_parses_the_xuid() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/xsts"))
        .and(header("accept", "application/json"))
        .and(body_partial_json(serde_json::json!({
            "Properties": { "SandboxId": "RETAIL", "UserTokens": ["xbl-token"] },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"Token":"xsts-token","DisplayClaims":{"xui":[{"uhs":"user-hash","xid":"2535"}]}}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let xsts = msa(&server)
        .xsts(&an_xbl_token())
        .await
        .expect("xsts succeeds");
    assert_eq!(xsts.token, "xsts-token");
    assert_eq!(xsts.uhs, "user-hash");
    assert_eq!(xsts.xuid.as_deref(), Some("2535"));
}

/// Serves one XSTS 401 body and returns the mapped error.
async fn xsts_401(body: &str) -> Error {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/xsts"))
        .respond_with(ResponseTemplate::new(401).set_body_string(body))
        .expect(1)
        .mount(&server)
        .await;
    msa(&server)
        .xsts(&an_xbl_token())
        .await
        .expect_err("a 401 fails")
}

#[tokio::test]
async fn xsts_maps_the_no_xbox_profile_code() {
    let err = xsts_401(r#"{"Identity":"0","XErr":2148916233,"Message":""}"#).await;
    assert!(matches!(err, Error::NoXboxProfile), "got {err:?}");
}

#[tokio::test]
async fn xsts_maps_every_known_code() {
    for (code, matches) in [
        (2148916235u64, "XboxRegionUnavailable"),
        (2148916236, "XboxAdultVerification"),
        (2148916237, "XboxAdultVerification"),
        (2148916238, "XboxChildAccount"),
    ] {
        let err = xsts_401(&format!(r#"{{"XErr":{code}}}"#)).await;
        let got = match err {
            Error::XboxRegionUnavailable => "XboxRegionUnavailable",
            Error::XboxAdultVerification => "XboxAdultVerification",
            Error::XboxChildAccount => "XboxChildAccount",
            other => panic!("{code} gave {other:?}"),
        };
        assert_eq!(got, matches, "for {code}");
    }
}

#[tokio::test]
async fn xsts_reports_an_unknown_code_as_is() {
    let err = xsts_401(r#"{"XErr":1234567}"#).await;
    assert!(matches!(err, Error::Xsts { code: 1234567 }), "got {err:?}");
}

#[tokio::test]
async fn xsts_reports_code_zero_when_the_body_has_none() {
    let err = xsts_401(r#"{"Message":"nope"}"#).await;
    assert!(matches!(err, Error::Xsts { code: 0 }), "got {err:?}");
}

#[tokio::test]
async fn mc_login_builds_the_identity_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mclogin"))
        .and(body_partial_json(serde_json::json!({
            "identityToken": "XBL3.0 x=user-hash;xsts-token",
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"access_token":"mc-token","expires_in":86400}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let token = msa(&server)
        .mc_login(&an_xsts_token())
        .await
        .expect("login succeeds");
    assert_eq!(token.access_token, "mc-token");
    assert_eq!(token.expires_in_secs, 86400);
}

#[tokio::test]
async fn mc_login_maps_403_to_invalid_app_registration() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mclogin"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;

    let err = msa(&server)
        .mc_login(&an_xsts_token())
        .await
        .expect_err("403 fails");
    assert!(matches!(err, Error::InvalidAppRegistration), "got {err:?}");
}

#[tokio::test]
async fn profile_sends_the_bearer_token_and_parses_the_answer() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .and(header("authorization", "Bearer mc-token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"id":"b50ad385829d3141a2167e7d7539ba7f","name":"Notch","skins":[]}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let profile = msa(&server)
        .profile("mc-token")
        .await
        .expect("profile succeeds");
    assert_eq!(
        profile,
        McProfile {
            id: "b50ad385829d3141a2167e7d7539ba7f".to_string(),
            name: "Notch".to_string(),
        }
    );
}

#[tokio::test]
async fn profile_maps_404_to_no_profile() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let err = msa(&server)
        .profile("mc-token")
        .await
        .expect_err("404 fails");
    assert!(matches!(err, Error::NoProfile), "got {err:?}");
}

#[test]
fn debug_hides_the_device_code() {
    let debug = format!("{:?}", a_device_code());
    assert!(!debug.contains("dev-secret"), "{debug}");
    assert!(debug.contains("<secret>"), "{debug}");
    assert!(debug.contains("ABCD-EFGH"), "{debug}");
}

#[test]
fn serializing_a_device_code_leaves_out_the_secret() {
    let json = serde_json::to_string(&a_device_code()).expect("serializes");
    assert!(!json.contains("dev-secret"), "{json}");
}

#[test]
fn debug_hides_both_msa_tokens() {
    let tokens = MsaTokens {
        access_token: "msa-access".to_string(),
        refresh_token: "msa-refresh".to_string(),
    };
    let debug = format!("{tokens:?}");
    assert!(!debug.contains("msa-access"), "{debug}");
    assert!(!debug.contains("msa-refresh"), "{debug}");
}

#[test]
fn debug_hides_the_xbox_and_minecraft_tokens() {
    let debug = format!("{:?}", an_xbl_token());
    assert!(!debug.contains("xbl-token"), "{debug}");
    let debug = format!("{:?}", an_xsts_token());
    assert!(!debug.contains("xsts-token"), "{debug}");
    let debug = format!(
        "{:?}",
        McToken {
            access_token: "mc-token".to_string(),
            expires_in_secs: 86400,
        }
    );
    assert!(!debug.contains("mc-token"), "{debug}");
}

#[test]
fn debug_hides_the_client_id() {
    let msa = Msa::new(client(), MsaEndpoints::default(), CLIENT_ID.to_string());
    let debug = format!("{msa:?}");
    assert!(!debug.contains(CLIENT_ID), "{debug}");
    assert!(debug.contains("<set>"), "{debug}");
}

#[test]
fn default_endpoints_are_the_production_urls() {
    let ep = MsaEndpoints::default();
    assert_eq!(
        ep.device_code,
        "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode"
    );
    assert_eq!(
        ep.token,
        "https://login.microsoftonline.com/consumers/oauth2/v2.0/token"
    );
    assert_eq!(ep.xbl, "https://user.auth.xboxlive.com/user/authenticate");
    assert_eq!(ep.xsts, "https://xsts.auth.xboxlive.com/xsts/authorize");
    assert_eq!(
        ep.mc_login,
        "https://api.minecraftservices.com/authentication/login_with_xbox"
    );
    assert_eq!(
        ep.profile,
        "https://api.minecraftservices.com/minecraft/profile"
    );
}
