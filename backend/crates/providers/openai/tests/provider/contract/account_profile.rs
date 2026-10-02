//! 账号级请求画像覆盖的应用与优先级契约。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gateway_core::account::OpaqueProviderData;
use gateway_core::engine::provider::Provider as _;
use gateway_core::engine::{
    AccountAttemptContext, AttemptContext, ModelRequestId, RequestAttemptContext,
};
use gateway_core::lifecycle::CancellationToken;
use gateway_core::policy::ClientApiKeyId;
use gateway_core::routing::{ClientRoutingScope, FrozenAccountScope, RuntimeAccount};
use provider_openai::CodexProvider;
use serde_json::{Map, Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{
    CAPTURE_COMPLETED_SSE, account_policy, create_account, planned_request, provider_with_base_url,
};
use crate::support::MemoryAccountStore;

const ACCOUNT_ID: &str = "acct_account_profile";

/// 单账号目录的冻结 scope；可选携带账号级画像覆盖。
fn scope_with_account_profile(
    override_profile: Option<OpaqueProviderData>,
) -> Arc<FrozenAccountScope> {
    let provider = gateway_core::routing::ProviderKind::new("openai").expect("provider");
    let accounts = BTreeMap::from([(
        gateway_core::account::ProviderAccountId::new(ACCOUNT_ID).expect("account"),
        RuntimeAccount::new(provider, BTreeSet::new()).with_request_profile(override_profile),
    )]);
    Arc::new(FrozenAccountScope::new(
        Arc::new(gateway_core::routing::RuntimeAccountDirectory::new(
            accounts,
        )),
        ClientRoutingScope::all_accounts(),
    ))
}

/// 请求级（Key/全局合并后的）已解析画像：与账号覆盖使用不同的终端标记。
fn request_level_profile(provider: &CodexProvider) -> OpaqueProviderData {
    let selection = OpaqueProviderData::new(
        serde_json::from_value::<Map<String, Value>>(json!({
            "client": "cli",
            "platform": "macos",
            "versionMode": "fixed",
            "codexVersion": "0.198.0",
            "terminal": "request-level-term",
        }))
        .expect("request level selection"),
    );
    provider
        .resolve_request_profile(&selection)
        .expect("request level profile resolves")
}

fn override_selection(terminal: &str) -> OpaqueProviderData {
    OpaqueProviderData::new(
        serde_json::from_value::<Map<String, Value>>(json!({
            "client": "cli",
            "platform": "linux",
            "versionMode": "fixed",
            "codexVersion": "0.199.0",
            "cliEntry": "tui",
            "terminal": terminal,
        }))
        .expect("account override selection"),
    )
}

/// 生成 operation 显式走 HTTP SSE；默认传输偏好会先尝试 WebSocket 升级。
fn http_generate_operation() -> gateway_core::operation::Operation {
    use gateway_core::operation::{GenerateRequest, ProtocolPayload};
    gateway_core::operation::Operation::Generate(GenerateRequest::from_protocol_payload(
        ProtocolPayload::json_object(
            "openai",
            serde_json::Map::from_iter([
                ("model".to_owned(), json!("gpt-5.4")),
                ("input".to_owned(), json!("hello")),
            ]),
        )
        .expect("OpenAI payload")
        .with_context(serde_json::Map::from_iter([(
            "use_websocket".to_owned(),
            json!(false),
        )])),
    ))
}

fn context(
    scope: Arc<FrozenAccountScope>,
    request_profile: Option<OpaqueProviderData>,
    request_id: &str,
) -> AttemptContext {
    AttemptContext::new(
        RequestAttemptContext::new(
            ModelRequestId::new(request_id).expect("request id"),
            ClientApiKeyId::new("key_account_profile").expect("client key id"),
        )
        .with_request_profile(request_profile)
        .with_disable_fast(false),
        std::num::NonZeroU32::new(1).expect("attempt"),
        SystemTime::now() + Duration::from_secs(30),
        account_policy(),
        AccountAttemptContext::new(BTreeSet::new(), None, None).with_account_scope(scope),
        None,
        CancellationToken::new(),
    )
}

#[tokio::test]
async fn account_override_wins_over_request_profile() {
    let store = Arc::new(MemoryAccountStore::default());
    create_account(&store, ACCOUNT_ID).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(CAPTURE_COMPLETED_SSE),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = provider_with_base_url(&store, server.uri());
    let request_profile = request_level_profile(&provider);
    let stream = provider
        .execute(
            planned_request("openai", http_generate_operation()),
            context(
                scope_with_account_profile(Some(override_selection("account-override-term"))),
                Some(request_profile),
                "req_account_override",
            ),
        )
        .await
        .expect("account override request starts");
    futures::StreamExt::for_each(stream, async |event| {
        event.expect("account override response event");
    })
    .await;

    let requests = server.received_requests().await.expect("captured requests");
    let user_agent = requests[0]
        .headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        user_agent.contains("account-override-term"),
        "user agent should carry the account override terminal: {user_agent}"
    );
    assert!(
        !user_agent.contains("request-level-term"),
        "request level profile must not override the account: {user_agent}"
    );
}

#[tokio::test]
async fn account_without_override_keeps_request_profile() {
    let store = Arc::new(MemoryAccountStore::default());
    create_account(&store, ACCOUNT_ID).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/codex/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(CAPTURE_COMPLETED_SSE),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = provider_with_base_url(&store, server.uri());
    let request_profile = request_level_profile(&provider);
    let stream = provider
        .execute(
            planned_request("openai", http_generate_operation()),
            context(
                scope_with_account_profile(None),
                Some(request_profile),
                "req_account_no_override",
            ),
        )
        .await
        .expect("request profile request starts");
    futures::StreamExt::for_each(stream, async |event| {
        event.expect("request profile response event");
    })
    .await;

    let requests = server.received_requests().await.expect("captured requests");
    let user_agent = requests[0]
        .headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        user_agent.contains("request-level-term"),
        "account without override must keep the request level profile: {user_agent}"
    );
}

#[tokio::test]
async fn invalid_account_override_fails_without_silent_fallback() {
    let store = Arc::new(MemoryAccountStore::default());
    create_account(&store, ACCOUNT_ID).await;
    let server = MockServer::start().await;
    // 非法预设：Desktop 不允许携带 cliEntry。
    let invalid = OpaqueProviderData::new(
        serde_json::from_value::<Map<String, Value>>(json!({
            "client": "desktop",
            "platform": "macos",
            "versionMode": "fixed",
            "codexVersion": "0.199.0",
            "desktopVersion": "26.1.0",
            "desktopBuild": "1",
            "cliEntry": "tui",
        }))
        .expect("invalid selection"),
    );
    let provider = provider_with_base_url(&store, server.uri());
    let error = match provider
        .execute(
            planned_request("openai", http_generate_operation()),
            context(
                scope_with_account_profile(Some(invalid)),
                None,
                "req_account_invalid_override",
            ),
        )
        .await
    {
        Ok(_) => panic!("invalid account override must fail the request"),
        Err(error) => error,
    };
    assert_eq!(
        error.kind(),
        gateway_core::error::ProviderErrorKind::InvalidRequest
    );
    assert!(
        server.received_requests().await.unwrap().is_empty(),
        "invalid override must not reach upstream with a silent fallback"
    );
}
