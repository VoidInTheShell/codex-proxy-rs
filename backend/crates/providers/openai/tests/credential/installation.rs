//! installation_id 出口分组派生的凭据链行为测试。
//!
//! 覆盖：导入（OAuth/API Key）按出口分组确定性派生、per-account 保持随机、
//! egress-grouped 缺派生器不静默回退、默认代理与文档 URL 同组共享、
//! 派生值通过凭据编解码校验（security.rs UUIDv4 约束）。
//! OAuth Create/Reauthorize 的派生与保留行为见 tests/credential/oauth.rs。

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures::future::BoxFuture;
use gateway_core::provider_ports::{
    ProviderInstallationIdStrategy, ProviderRefreshPolicy, ProviderRuntimePolicyPort,
    ProviderStoreError,
};
use provider_openai::credential::{
    CodexCredentialAdminError, CodexCredentialAdminService, CodexCredentialCodec,
    CodexInstallationIdDeriver,
};

use crate::support::{TestLeaseCoordinator, runtime_policy};

/// 可配置 installation_id 策略的运行时策略 fake。
struct StaticInstallationIdStrategy(ProviderInstallationIdStrategy);

impl ProviderRuntimePolicyPort for StaticInstallationIdStrategy {
    fn load_refresh_policy(
        &self,
    ) -> BoxFuture<'_, Result<ProviderRefreshPolicy, ProviderStoreError>> {
        Box::pin(async {
            ProviderRefreshPolicy::try_new(
                Duration::from_secs(60 * 60),
                NonZeroU32::new(2).expect("positive concurrency"),
            )
        })
    }

    fn load_installation_id_strategy(
        &self,
    ) -> BoxFuture<'_, Result<ProviderInstallationIdStrategy, ProviderStoreError>> {
        Box::pin(async { Ok(self.0) })
    }
}

fn strategy_port(strategy: ProviderInstallationIdStrategy) -> Arc<dyn ProviderRuntimePolicyPort> {
    Arc::new(StaticInstallationIdStrategy(strategy))
}

fn deriver() -> CodexInstallationIdDeriver {
    // 独立密钥文件：与生产 identity_hmac_secret 及其他测试隔离。
    let path = std::env::temp_dir().join(format!(
        "codex-installation-egress-test-{}",
        std::process::id()
    ));
    CodexInstallationIdDeriver::load_or_create(&path).expect("deriver secret")
}

fn test_jwt(user_id: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(serde_json::json!({"alg": "RS256"}).to_string());
    let payload = URL_SAFE_NO_PAD.encode(
        serde_json::json!({
            "sub": user_id,
            "https://api.openai.com/auth": {"chatgpt_user_id": user_id}
        })
        .to_string(),
    );
    format!("{header}.{payload}.unverified-signature")
}

fn decoded_installation_id(account: &gateway_core::account::NewProviderAccount) -> String {
    let runtime = CodexCredentialCodec::decode(&account.credential).expect("decode credential");
    runtime.installation_id
}

struct UnusedRefresher;

#[async_trait]
impl provider_openai::credential::token_client::TokenRefresher for UnusedRefresher {
    async fn refresh(
        &self,
        _refresh_token: &str,
    ) -> Result<
        provider_openai::credential::token_client::TokenPair,
        provider_openai::credential::token_client::RefreshFailure,
    > {
        panic!("installation_id derivation tests must not refresh tokens")
    }
}

fn egress_grouped_service() -> CodexCredentialAdminService {
    CodexCredentialAdminService::new(
        Arc::new(UnusedRefresher),
        Arc::new(TestLeaseCoordinator::default()),
        strategy_port(ProviderInstallationIdStrategy::EgressGrouped),
    )
    .with_installation_derivation(deriver())
}

#[tokio::test]
async fn egress_grouped_import_shares_installation_id_within_same_proxy_group() {
    let service = egress_grouped_service();
    let prepared = service
        .prepare_import_document(serde_json::json!({"data": {
        "accounts": [
            {"name": "a", "platform": "openai", "type": "oauth", "proxy_key": "shared", "credentials": {"access_token": test_jwt("user-a")}},
            {"name": "b", "platform": "openai", "type": "oauth", "proxy_key": "shared", "credentials": {"access_token": test_jwt("user-b")}}
        ],
        "proxies": [
            {"proxy_key": "shared", "protocol": "http", "host": "127.0.0.1", "port": 18080, "status": "active"}
        ]
    }}))
        .await
        .expect("import document");
    assert_eq!(prepared.accounts().len(), 2);
    let first = decoded_installation_id(&prepared.accounts()[0]);
    let second = decoded_installation_id(&prepared.accounts()[1]);
    assert_eq!(
        first, second,
        "same egress group must share installation_id"
    );
}

#[tokio::test]
async fn egress_grouped_import_differs_across_direct_and_proxy_groups() {
    let service = egress_grouped_service();
    let prepared = service
        .prepare_import_document(serde_json::json!({"data": {
        "accounts": [
            {"name": "direct-a", "platform": "openai", "type": "oauth", "credentials": {"access_token": test_jwt("user-a")}},
            {"name": "direct-b", "platform": "openai", "type": "oauth", "credentials": {"access_token": test_jwt("user-b")}},
            {"name": "proxy-a", "platform": "openai", "type": "oauth", "proxy_key": "p", "credentials": {"access_token": test_jwt("user-c")}}
        ],
        "proxies": [
            {"proxy_key": "p", "protocol": "http", "host": "127.0.0.1", "port": 18080, "status": "active"}
        ]
    }}))
        .await
        .expect("import document");
    assert_eq!(prepared.accounts().len(), 3);
    let direct_a = decoded_installation_id(&prepared.accounts()[0]);
    let direct_b = decoded_installation_id(&prepared.accounts()[1]);
    let proxy_a = decoded_installation_id(&prepared.accounts()[2]);
    assert_eq!(direct_a, direct_b, "direct accounts share one group");
    assert_ne!(direct_a, proxy_a, "direct and proxy groups differ");
}

#[tokio::test]
async fn egress_grouped_import_applies_to_api_key_accounts() {
    let service = egress_grouped_service();
    let prepared = service
        .prepare_import_document(serde_json::json!({
        "accounts": [
            {"name": "key-a", "authentication_kind": "api_key", "outboundProxyUrl": "http://127.0.0.1:18080", "credentials": {"base_url": "https://api.example.com", "api_key": "sk-test-a"}},
            {"name": "key-b", "authentication_kind": "api_key", "outboundProxyUrl": "http://127.0.0.1:18080", "credentials": {"base_url": "https://api.example.com", "api_key": "sk-test-b"}},
            {"name": "key-direct", "authentication_kind": "api_key", "credentials": {"base_url": "https://api.example.com", "api_key": "sk-test-c"}}
        ]
    }))
        .await
        .expect("import document");
    assert_eq!(prepared.accounts().len(), 3);
    let key_a = decoded_installation_id(&prepared.accounts()[0]);
    let key_b = decoded_installation_id(&prepared.accounts()[1]);
    let key_direct = decoded_installation_id(&prepared.accounts()[2]);
    assert_eq!(key_a, key_b);
    assert_ne!(key_a, key_direct);
}

#[tokio::test]
async fn egress_grouped_import_matches_explicit_derivation_for_group() {
    let service = egress_grouped_service();
    let prepared = service
        .prepare_import_document(serde_json::json!({
        "accounts": [
            {"name": "a", "platform": "openai", "type": "oauth", "proxy_key": "p", "credentials": {"access_token": test_jwt("user-a")}}
        ],
        "proxies": [
            {"proxy_key": "p", "protocol": "http", "host": "127.0.0.1", "port": 18080, "status": "active"}
        ]
    }))
        .await
        .expect("import document");
    let imported = decoded_installation_id(&prepared.accounts()[0]);
    // 组键是代理完整 URL 的规范化形式（Url 解析补全路径 "/"）。
    let normalized = gateway_core::account::OutboundProxy::parse("http://127.0.0.1:18080")
        .expect("proxy")
        .expose_url()
        .to_owned();
    let expected = deriver().derive_for_group(&normalized);
    assert_eq!(imported, expected);
}

#[tokio::test]
async fn per_account_import_keeps_independent_installation_ids() {
    let service = CodexCredentialAdminService::new(
        Arc::new(UnusedRefresher),
        Arc::new(TestLeaseCoordinator::default()),
        strategy_port(ProviderInstallationIdStrategy::PerAccount),
    )
    .with_installation_derivation(deriver());
    let prepared = service
        .prepare_import_document(serde_json::json!({"data": {
        "accounts": [
            {"name": "a", "platform": "openai", "type": "oauth", "proxy_key": "shared", "credentials": {"access_token": test_jwt("user-a")}},
            {"name": "b", "platform": "openai", "type": "oauth", "proxy_key": "shared", "credentials": {"access_token": test_jwt("user-b")}}
        ],
        "proxies": [
            {"proxy_key": "shared", "protocol": "http", "host": "127.0.0.1", "port": 18080, "status": "active"}
        ]
    }}))
        .await
        .expect("import document");
    let first = decoded_installation_id(&prepared.accounts()[0]);
    let second = decoded_installation_id(&prepared.accounts()[1]);
    assert_ne!(first, second, "per-account keeps independent identities");
}

#[tokio::test]
async fn per_account_without_deriver_keeps_legacy_random_behavior() {
    // 既有装配（未注入派生器）在 per-account 下保持历史随机行为。
    let service = CodexCredentialAdminService::new(
        Arc::new(UnusedRefresher),
        Arc::new(TestLeaseCoordinator::default()),
        runtime_policy(),
    );
    let prepared = service
        .prepare_import_document(serde_json::json!({
        "accounts": [
            {"name": "a", "platform": "openai", "type": "oauth", "credentials": {"access_token": test_jwt("user-a")}}
        ]
    }))
        .await
        .expect("import document");
    let value = decoded_installation_id(&prepared.accounts()[0]);
    assert_eq!(
        uuid::Uuid::parse_str(&value)
            .expect("uuid")
            .get_version_num(),
        4
    );
}

#[tokio::test]
async fn egress_grouped_without_deriver_fails_instead_of_silent_random() {
    let service = CodexCredentialAdminService::new(
        Arc::new(UnusedRefresher),
        Arc::new(TestLeaseCoordinator::default()),
        strategy_port(ProviderInstallationIdStrategy::EgressGrouped),
    );
    let error = service
        .prepare_import_document(serde_json::json!({
        "accounts": [
            {"name": "a", "platform": "openai", "type": "oauth", "credentials": {"access_token": test_jwt("user-a")}}
        ]
    }))
        .await
        .expect_err("egress-grouped without deriver must fail");
    assert!(matches!(
        error,
        CodexCredentialAdminError::InstallationPolicyUnavailable
    ));
}

#[tokio::test]
async fn egress_grouped_derivation_reuses_default_proxy_group() {
    // 默认代理（Saved 记录解析出的 URL）与文档 URL 指向同一出口时共享派生值。
    let service = egress_grouped_service();
    let default_proxy =
        gateway_core::account::OutboundProxy::parse("http://127.0.0.1:18080").expect("proxy");
    let prepared = service
        .prepare_import_document_with_proxy(
            serde_json::json!({
        "accounts": [
            {"name": "default-bound", "platform": "openai", "type": "oauth", "credentials": {"access_token": test_jwt("user-a")}},
            {"name": "doc-bound", "platform": "openai", "type": "oauth", "proxy_key": "p", "credentials": {"access_token": test_jwt("user-b")}}
        ],
        "proxies": [
            {"proxy_key": "p", "protocol": "http", "host": "127.0.0.1", "port": 18080, "status": "active"}
        ]
    }),
            Some(&default_proxy),
        )
        .await
        .expect("import document");
    let default_bound = decoded_installation_id(&prepared.accounts()[0]);
    let doc_bound = decoded_installation_id(&prepared.accounts()[1]);
    assert_eq!(
        default_bound, doc_bound,
        "same egress via default proxy and document URL must share installation_id"
    );
}

// —— 单元级行为（从 src/credential/installation.rs 内联模块迁出，仓库规则：生产源码不承载测试）——
use gateway_core::account::OutboundProxy;

mod unit {
    use super::*;
    use uuid::Uuid;


    
        
    
    fn deriver() -> CodexInstallationIdDeriver {
        CodexInstallationIdDeriver::load_or_create(std::path::Path::new(
            "/tmp/codex-installation-id-deriver-test-secret",
        ))
        .expect("identity")
    }

    fn proxy(url: &str) -> OutboundProxy {
        OutboundProxy::parse(url).expect("proxy url")
    }

    #[test]
    fn group_key_distinguishes_direct_and_proxy_exits() {
        assert_eq!(CodexInstallationIdDeriver::group_key(None), "direct");
        // 组键是代理 URL 的规范化形式： Url 解析补全路径，不同写法归一到同组。
        let proxy = proxy("http://user:pass@127.0.0.1:18080");
        assert_eq!(
            CodexInstallationIdDeriver::group_key(Some(&proxy)),
            "http://user:pass@127.0.0.1:18080/"
        );
    }

    #[test]
    fn egress_grouped_derivation_is_deterministic_per_group() {
        let deriver = deriver();
        let first = deriver.derive_for_group("direct");
        let second = deriver.derive_for_group("direct");
        assert_eq!(first, second);
        // 不同部署密钥必须派生不同值：换一个密钥文件隔离验证。
        let other = CodexInstallationIdDeriver::load_or_create(std::path::Path::new(
            "/tmp/codex-installation-id-deriver-test-secret-2",
        ))
        .expect("identity");
        assert_ne!(first, other.derive_for_group("direct"));
    }

    #[test]
    fn egress_grouped_derivation_differs_across_groups() {
        let deriver = deriver();
        let direct = deriver.derive_for_group("direct");
        let proxy_a = deriver.derive_for_group("http://127.0.0.1:18080");
        let proxy_b = deriver.derive_for_group("socks5h://127.0.0.1:1080");
        assert_ne!(direct, proxy_a);
        assert_ne!(proxy_a, proxy_b);
        assert_ne!(direct, proxy_b);
    }

    #[test]
    fn derived_installation_ids_pass_uuid_v4_validation() {
        let deriver = deriver();
        for group_key in ["direct", "http://127.0.0.1:18080", "socks5h://[::1]:1080"] {
            let value = deriver.derive_for_group(group_key);
            let uuid = Uuid::parse_str(&value).expect("uuid v4 parse");
            assert_eq!(uuid.get_version_num(), 4);
            assert_eq!(value, uuid.hyphenated().to_string());
        }
    }

    #[test]
    fn per_account_strategy_keeps_random_generation() {
        let deriver = deriver();
        let first = deriver
            .derive_observed(ProviderInstallationIdStrategy::PerAccount, None)
            .value;
        let second = deriver
            .derive_observed(ProviderInstallationIdStrategy::PerAccount, None)
            .value;
        assert_ne!(first, second);
        for value in [first, second] {
            assert_eq!(Uuid::parse_str(&value).expect("uuid").get_version_num(), 4);
        }
    }

    #[test]
    fn egress_grouped_strategy_follows_proxy_group() {
        let deriver = deriver();
        let proxy = proxy("http://127.0.0.1:18080");
        let via_derive = deriver
            .derive_observed(ProviderInstallationIdStrategy::EgressGrouped, Some(&proxy))
            .value;
        assert_eq!(
            via_derive,
            deriver.derive_for_group("http://127.0.0.1:18080/")
        );
        let direct = deriver
            .derive_observed(ProviderInstallationIdStrategy::EgressGrouped, None)
            .value;
        assert_eq!(direct, deriver.derive_for_group("direct"));
        assert_ne!(direct, via_derive);
    }

    #[test]
    fn derive_observed_reports_strategy_and_fingerprint() {
        let deriver = deriver();
        let grouped = deriver.derive_observed(
            ProviderInstallationIdStrategy::EgressGrouped,
            Some(&proxy("http://127.0.0.1:18080")),
        );
        assert_eq!(
            grouped.strategy,
            ProviderInstallationIdStrategy::EgressGrouped
        );
        assert_eq!(
            grouped.fingerprint,
            CodexInstallationIdDeriver::group_fingerprint("http://127.0.0.1:18080/")
        );
    }

    #[test]
    fn group_fingerprint_is_stable_and_non_reversible_prefix() {
        assert_eq!(
            CodexInstallationIdDeriver::group_fingerprint("direct"),
            CodexInstallationIdDeriver::group_fingerprint("direct")
        );
        assert_ne!(
            CodexInstallationIdDeriver::group_fingerprint("direct"),
            CodexInstallationIdDeriver::group_fingerprint("http://127.0.0.1:18080")
        );
        assert_eq!(
            CodexInstallationIdDeriver::group_fingerprint("direct").len(),
            8
        );
    }

    /// 策略读取失败必须让操作失败而不是静默回退 per-account；
    /// 这里验证端口语义：默认实现返回 PerAccount。
    #[tokio::test]
    async fn runtime_policy_default_strategy_is_per_account() {
        struct DefaultPolicy;
        impl ProviderRuntimePolicyPort for DefaultPolicy {
            fn load_refresh_policy(
                &self,
            ) -> BoxFuture<'_, Result<ProviderRefreshPolicy, ProviderStoreError>> {
                Box::pin(async {
                    ProviderRefreshPolicy::try_new(
                        std::time::Duration::from_secs(60),
                        NonZeroU32::new(2).expect("positive"),
                    )
                })
            }
        }
        let strategy = DefaultPolicy
            .load_installation_id_strategy()
            .await
            .expect("default strategy");
        assert_eq!(strategy, ProviderInstallationIdStrategy::PerAccount);
    }

    #[test]
    fn strategy_values_round_trip_through_parse() {
        for strategy in [
            ProviderInstallationIdStrategy::PerAccount,
            ProviderInstallationIdStrategy::EgressGrouped,
        ] {
            assert_eq!(
                ProviderInstallationIdStrategy::parse(strategy.as_str()),
                Some(strategy)
            );
        }
        assert_eq!(ProviderInstallationIdStrategy::parse("per_account"), None);
        assert_eq!(ProviderInstallationIdStrategy::parse(""), None);
    }
}
