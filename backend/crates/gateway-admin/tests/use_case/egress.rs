//! 出口分组视图回归：直连/代理分组、共享标记与阈值判定。

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use gateway_admin::{
    model::{
        MutationContext, Revision,
        proxies::{
            EgressAccountBinding, EgressDistribution, EgressFacts, EgressGroupKind,
            EgressProxyFact, NewProxy, ProxyAccountListQuery, ProxyAccountPage, ProxyListQuery,
            ProxyMutation, ProxyPage, ProxyRecord, ProxyTestResult, UpdateProxy,
        },
        settings::{AdminApiKey, AdminApiKeyMutation, ReplaceRuntimeSettings, RuntimeSettings},
    },
    ports::{
        proxy::{ProxyProbe, ProxyStore},
        store::{AdminStoreResult, SettingsStore},
    },
};
use gateway_core::account::{OutboundProxy, ProviderAccountId};

use super::AdminHarness;

/// 构造出口分布测试事实：直连×2、共享同一代理×2、独占另一代理×1、零绑定代理×1。
fn facts() -> EgressFacts {
    EgressFacts {
        accounts: vec![
            binding("acct_d2", "直连二号", None),
            binding("acct_d1", "直连一号", None),
            binding("acct_s2", "共享二号", Some("proxy_shared")),
            binding("acct_s1", "共享一号", Some("proxy_shared")),
            binding("acct_solo", "独占账号", Some("proxy_solo")),
        ],
        proxies: vec![
            EgressProxyFact {
                id: "proxy_shared".to_owned(),
                name: "共享代理".to_owned(),
                endpoint: "http://proxy.shared.example:8080".to_owned(),
                location: Some(location()),
                exit_ip: Some("203.0.113.2".parse().unwrap()),
            },
            EgressProxyFact {
                id: "proxy_solo".to_owned(),
                name: "独占代理".to_owned(),
                endpoint: "http://proxy.solo.example:1080".to_owned(),
                location: None,
                exit_ip: None,
            },
            // 未绑定任何账号的代理不进入账号 × 出口分布。
            EgressProxyFact {
                id: "proxy_unused".to_owned(),
                name: "未使用代理".to_owned(),
                endpoint: "http://proxy.unused.example:1080".to_owned(),
                location: None,
                exit_ip: None,
            },
        ],
    }
}

fn binding(id: &str, name: &str, proxy_id: Option<&str>) -> EgressAccountBinding {
    EgressAccountBinding {
        id: id.to_owned(),
        name: name.to_owned(),
        provider_kind: "openai".to_owned(),
        enabled: true,
        proxy_id: proxy_id.map(str::to_owned),
    }
}

fn location() -> gateway_core::account::RequestLocation {
    serde_json::from_value(serde_json::json!({
        "country": "JP", "region": "Tokyo", "city": "Tokyo", "timezone": "Asia/Tokyo"
    }))
    .unwrap()
}

fn runtime_settings(alert_enabled: bool, alert_threshold: u32) -> RuntimeSettings {
    RuntimeSettings {
        request_profiles: Default::default(),
        request_location_enabled: false,
        request_location: Default::default(),
        config_revision: Revision::new(1).unwrap(),
        model_mappings: Default::default(),
        refresh_margin_seconds: 300,
        refresh_concurrency: 2,
        max_concurrent_per_account: 5,
        request_interval_ms: 0,
        max_waiting_per_key: 0,
        max_waiting_per_account: 0,
        concurrency_wait_timeout_seconds: 30,
        openai_guardian_reserved_concurrency: 0,
        responses_max_decompressed_body_bytes: 64 * 1024 * 1024,
        smart_scheduling: gateway_core::account::SmartSchedulingConfig::default(),
        rotation_strategy: gateway_admin::model::settings::RotationStrategy::Smart,
        min_codex_desktop_version: None,
        min_codex_cli_version: None,
        usage_retention_days: 31,
        ops_event_retention_days: 30,
        audit_retention_days: 90,
        account_auto_freeze_enabled: false,
        account_auto_freeze_threshold: 12,
        account_auto_freeze_window_seconds: 600,
        account_auto_freeze_duration_seconds: 7_200,
        account_auto_freeze_probe_enabled: true,
        account_auto_freeze_probe_model: None,
        account_auto_freeze_adaptive_concurrency: true,
        account_warmup_enabled: false,
        account_warmup_schedule_time: "08:00".to_owned(),
        account_warmup_model: None,
        egress_sharing_alert_enabled: alert_enabled,
        egress_sharing_alert_threshold: alert_threshold,
        openai_installation_id_strategy:
            gateway_core::provider_ports::ProviderInstallationIdStrategy::PerAccount,
        updated_at: Utc::now(),
    }
}

struct StaticEgressStore {
    facts: EgressFacts,
    settings: RuntimeSettings,
}

#[async_trait]
impl ProxyStore for StaticEgressStore {
    async fn reserve_import(
        &self,
        _: &str,
    ) -> AdminStoreResult<gateway_admin::ports::proxy::ProxyImportReservation> {
        panic!("unexpected proxy reserve")
    }
    async fn list(&self, _: ProxyListQuery) -> AdminStoreResult<ProxyPage> {
        panic!("unexpected proxy list")
    }
    async fn egress_facts(&self) -> AdminStoreResult<EgressFacts> {
        Ok(self.facts.clone())
    }
    async fn list_accounts(&self, _: ProxyAccountListQuery) -> AdminStoreResult<ProxyAccountPage> {
        panic!("unexpected proxy account list")
    }
    async fn get(&self, _: &str) -> AdminStoreResult<ProxyRecord> {
        panic!("unexpected proxy get")
    }
    async fn remove_account(
        &self,
        _: &str,
        _: &ProviderAccountId,
        _: &MutationContext,
    ) -> AdminStoreResult<Revision> {
        panic!("unexpected proxy account removal")
    }
    async fn create(&self, _: NewProxy, _: &MutationContext) -> AdminStoreResult<ProxyMutation> {
        panic!("unexpected proxy create")
    }
    async fn update(&self, _: UpdateProxy, _: &MutationContext) -> AdminStoreResult<ProxyMutation> {
        panic!("unexpected proxy update")
    }
    async fn delete(
        &self,
        _: &str,
        _: Revision,
        _: &MutationContext,
    ) -> AdminStoreResult<Revision> {
        panic!("unexpected proxy delete")
    }
    async fn record_test(
        &self,
        _: &str,
        _: Revision,
        _: ProxyTestResult,
        _: &MutationContext,
    ) -> AdminStoreResult<ProxyMutation> {
        panic!("unexpected proxy test")
    }
}

#[async_trait]
impl ProxyProbe for StaticEgressStore {
    async fn test(&self, _: &OutboundProxy, _: bool) -> ProxyTestResult {
        panic!("unexpected proxy probe")
    }
}

#[async_trait]
impl SettingsStore for StaticEgressStore {
    async fn load_pricing(&self) -> AdminStoreResult<gateway_admin::model::pricing::StoredPricing> {
        Ok(Default::default())
    }
    async fn sync_pricing(
        &self,
        _: gateway_admin::model::pricing::PricingSyncChanges,
        _: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::Revision> {
        panic!("unexpected pricing sync")
    }
    async fn update_pricing(
        &self,
        _: gateway_admin::model::pricing::UpdatePricing,
        _: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::Revision> {
        panic!("unexpected pricing update")
    }
    async fn load_runtime_settings(&self) -> AdminStoreResult<RuntimeSettings> {
        Ok(self.settings.clone())
    }
    async fn admin_api_key_exists(&self) -> AdminStoreResult<bool> {
        Ok(false)
    }
    async fn replace_runtime_settings(
        &self,
        _: ReplaceRuntimeSettings,
        _: &MutationContext,
    ) -> AdminStoreResult<RuntimeSettings> {
        panic!("unexpected settings replace")
    }
    async fn replace_admin_api_key(
        &self,
        _: AdminApiKey,
        _: &MutationContext,
    ) -> AdminStoreResult<AdminApiKeyMutation> {
        panic!("unexpected admin api key replace")
    }
    async fn delete_admin_api_key(
        &self,
        _: &MutationContext,
    ) -> AdminStoreResult<AdminApiKeyMutation> {
        panic!("unexpected admin api key delete")
    }
}

async fn distribution(alert_enabled: bool, alert_threshold: u32) -> EgressDistribution {
    let store = Arc::new(StaticEgressStore {
        facts: facts(),
        settings: runtime_settings(alert_enabled, alert_threshold),
    });
    let services = AdminHarness::new()
        .proxies(store.clone())
        .settings(store)
        .build()
        .await;
    services.proxies().egress_distribution().await.unwrap()
}

fn assert_account_names(group: &gateway_admin::model::proxies::EgressGroup, expected: &[&str]) {
    let names: Vec<&str> = group.accounts.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, expected, "组内账号应按名称稳定排序");
}

#[tokio::test]
async fn egress_distribution_groups_direct_and_proxy_egress() {
    let result = distribution(true, 2).await;
    // 风险优先（账号数降序），同数时直连在前；零绑定代理不出现。
    let groups: Vec<_> = result
        .groups
        .iter()
        .map(|group| (group.kind, group.name.as_deref(), group.account_count))
        .collect();
    assert_eq!(
        groups,
        vec![
            (EgressGroupKind::Direct, None, 2),
            (EgressGroupKind::Proxy, Some("共享代理"), 2),
            (EgressGroupKind::Proxy, Some("独占代理"), 1),
        ]
    );
    let direct = &result.groups[0];
    assert_eq!(direct.proxy_id, None);
    assert_eq!(direct.endpoint, None);
    assert_eq!(direct.location, None);
    assert_eq!(direct.exit_ip, None);
    assert_account_names(direct, &["直连一号", "直连二号"]);

    let shared = &result.groups[1];
    assert_eq!(shared.proxy_id.as_deref(), Some("proxy_shared"));
    assert_eq!(
        shared.endpoint.as_deref(),
        Some("http://proxy.shared.example:8080")
    );
    assert_eq!(shared.location.as_ref(), Some(&location()));
    assert_eq!(
        shared.exit_ip.map(|ip| ip.to_string()).as_deref(),
        Some("203.0.113.2")
    );
    assert_account_names(shared, &["共享一号", "共享二号"]);
}

#[tokio::test]
async fn egress_distribution_marks_groups_reaching_threshold() {
    let result = distribution(true, 2).await;
    // 达到阈值（含直连组）标记，未达到不标记。
    assert!(result.groups[0].alerting, "直连组达到阈值应标记");
    assert!(result.groups[1].alerting, "共享代理组达到阈值应标记");
    assert!(!result.groups[2].alerting, "单账号独占组不应标记");
    assert!(result.alert_enabled);
    assert_eq!(result.alert_threshold, 2);

    // 阈值调高后只有更大的组标记。
    let raised = distribution(true, 3).await;
    assert!(
        !raised.groups.iter().any(|group| group.alerting),
        "无组达到阈值 3 时不应标记"
    );
}

#[tokio::test]
async fn egress_distribution_keeps_groups_when_alert_disabled() {
    let result = distribution(false, 2).await;
    assert!(!result.alert_enabled);
    assert_eq!(result.alert_threshold, 2);
    // 关闭提醒只停用标记，分组与账号事实照常返回，视图保持可见。
    assert_eq!(result.groups.len(), 3);
    assert!(result.groups.iter().all(|group| !group.alerting));
    assert_eq!(result.groups[1].account_count, 2);
}

#[tokio::test]
async fn egress_distribution_returns_empty_groups_without_accounts() {
    let store = Arc::new(StaticEgressStore {
        facts: EgressFacts::default(),
        settings: runtime_settings(true, 2),
    });
    let services = AdminHarness::new()
        .proxies(store.clone())
        .settings(store)
        .build()
        .await;
    let distribution = services.proxies().egress_distribution().await.unwrap();
    assert!(distribution.groups.is_empty());
}
