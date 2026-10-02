//! installation_id 派生策略与出口分组派生。
//!
//! 官方语义（/tmp/codex-analysis/codex @ ca466061）：installation_id 是
//! CODEX_HOME 安装级 UUIDv4，绑定安装而不绑定账号或网络；同一台设备切换账号
//! 共享同一 installation_id，设备换网络不变。`egress-grouped` 策略据此把
//! 「一个出口分组」（直连一组、同一 outbound_proxies 记录一组）在上游视角
//! 呈现为一台设备：凭据创建时按账号当时的出口分组，用部署级密钥确定性派生，
//! 同组账号共享同一 installation_id。
//!
//! 派生只在凭据创建时发生一次；之后换绑代理不重算（设备换网络语义），
//! 存量凭据也不会因策略切换被重写（迁移语义见
//! .pi/coordination/P2-3-DESIGN.md）。

use std::path::Path;
use std::sync::Arc;

use gateway_core::account::OutboundProxy;
use gateway_core::provider_ports::{ProviderInstallationIdStrategy, ProviderRuntimePolicyPort};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::transport::session::{CodexSessionIdentity, CodexSessionIdentityError};

/// 域分离标签；与 `transport::session` 的 `local-conversation` 会话锚点
/// 派生共用部署密钥但互不重叠。
const INSTALLATION_ID_DOMAIN: &[u8] = b"openai/installation-id/egress-group/v1\0";

/// 直连（无代理）账号的固定组键；代理 URL 必带 scheme 前缀，不会与之相等。
const DIRECT_GROUP_KEY: &str = "direct";

/// 派生器初始化失败；密钥文件不可读写或内容非法。
#[derive(Debug, thiserror::Error)]
pub enum CodexInstallationIdDeriverError {
    #[error("installation id deriver secret is unavailable")]
    SecretUnavailable,
}

impl From<CodexSessionIdentityError> for CodexInstallationIdDeriverError {
    fn from(_: CodexSessionIdentityError) -> Self {
        Self::SecretUnavailable
    }
}

/// 出口分组派生器；持有部署级 HMAC 密钥（`identity_hmac_secret`）。
///
/// 密钥是持久运行数据：更换或丢失只影响之后的派生（同组会派生出不同的
/// UUID），已存凭据永不重算。Clone 共享同一密钥字节。
#[derive(Clone)]
pub struct CodexInstallationIdDeriver {
    identity: CodexSessionIdentity,
}

/// 派生结果的观测投影：携带生成点日志与账号对账所需字段，
/// 不含组键明文（代理 URL 可内嵌认证）。
pub struct DerivedInstallationId {
    pub value: String,
    pub strategy: ProviderInstallationIdStrategy,
    pub fingerprint: String,
}

impl CodexInstallationIdDeriver {
    /// 从运行数据目录的密钥文件加载（首次创建）派生器；供独立装配与测试使用，
    /// 生产组装经 `OpenAiConfig::session_identity` 复用同一份密钥。
    pub fn load_or_create(path: &Path) -> Result<Self, CodexInstallationIdDeriverError> {
        Ok(Self {
            identity: CodexSessionIdentity::load_or_create(path)?,
        })
    }

    pub(crate) fn new(identity: CodexSessionIdentity) -> Self {
        Self { identity }
    }

    /// 按策略生成 installation_id，并携带生成点日志所需的观测字段。
    ///
    /// `PerAccount` 保持历史行为（每账号独立随机 UUIDv4）；`EgressGrouped`
    /// 按账号创建时的出口分组确定性派生，同组账号共享同一值。
    pub fn derive_observed(
        &self,
        strategy: ProviderInstallationIdStrategy,
        proxy: Option<&OutboundProxy>,
    ) -> DerivedInstallationId {
        let group_key = Self::group_key(proxy);
        DerivedInstallationId {
            value: match strategy {
                ProviderInstallationIdStrategy::PerAccount => Uuid::new_v4().to_string(),
                ProviderInstallationIdStrategy::EgressGrouped => self.derive_for_group(&group_key),
            },
            strategy,
            fingerprint: Self::group_fingerprint(&group_key),
        }
    }

    /// egress-grouped 下给定组键的确定性派生值；也用于账号对账比较
    /// （存储值是否等于当前出口分组的派生值）。
    pub fn derive_for_group(&self, group_key: &str) -> String {
        let digest = self
            .identity
            .hmac(&[INSTALLATION_ID_DOMAIN, group_key.as_bytes()]);
        // 前 16 字节强制 UUID v4 版本位与 RFC4122 variant 位，保持
        // credential/security.rs 的 UUIDv4 校验与 oauth pending 校验兼容。
        let mut bytes = [0_u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid::from_bytes(bytes).hyphenated().to_string()
    }

    /// 出口分组键：直连为固定键，代理为 URL 字符串。
    ///
    /// 系统所有代理登记路径（导入/授权提交的 `ensure_proxy_for_url` 与手工
    /// create）都按完整 URL 去重，URL 与 outbound_proxies 记录 1:1，因此
    /// URL 键与按记录 ID 分组（P2-2 的分组概念）在稳态下分区一致；记录 ID
    /// 在 Provider 派生时点对文档 URL/授权 URL 路径不可得，无法统一使用。
    /// 管理员修改代理记录 URL 后新凭据按新 URL 派生、旧凭据保持原值，
    /// 退化为「同一网络多台设备」的官方正常分布。
    pub fn group_key(proxy: Option<&OutboundProxy>) -> String {
        match proxy {
            None => DIRECT_GROUP_KEY.to_owned(),
            Some(proxy) => proxy.expose_url().to_owned(),
        }
    }

    /// 组键的非敏感指纹（SHA-256 前 8 hex）：同组账号指纹相同，用于日志与
    /// 管理端对账；不暴露代理 URL 明文（URL 可内嵌代理认证）。
    pub fn group_fingerprint(group_key: &str) -> String {
        let digest = Sha256::digest(group_key.as_bytes());
        hex::encode(&digest[..4])
    }
}

/// 账号管理面的 installation_id 派生上下文：策略端口 + 派生器。
///
/// 供账号详情的 `credentialConfiguration.installationId` 对账报告使用：
/// 呈现当前策略、账号当前出口分组指纹，以及存储值是否等于该分组的确定性
/// 派生值（per-account 历史账号与换绑代理后的账号为 false，均属预期，
/// 不是告警状态）。
pub(crate) struct CodexInstallationAdminContext {
    policy: Arc<dyn ProviderRuntimePolicyPort>,
    deriver: CodexInstallationIdDeriver,
}

impl CodexInstallationAdminContext {
    pub(crate) fn new(
        policy: Arc<dyn ProviderRuntimePolicyPort>,
        deriver: CodexInstallationIdDeriver,
    ) -> Self {
        Self { policy, deriver }
    }

    /// 生成账号级 installation_id 对账报告；不回显 installation_id 值与组键明文。
    pub(crate) async fn report(
        &self,
        stored_installation_id: &str,
        proxy: Option<&OutboundProxy>,
    ) -> Result<serde_json::Value, gateway_core::provider_ports::ProviderStoreError> {
        let strategy = self.policy.load_installation_id_strategy().await?;
        let group_key = CodexInstallationIdDeriver::group_key(proxy);
        let matches_current_group =
            self.deriver.derive_for_group(&group_key) == stored_installation_id;
        Ok(serde_json::json!({
            "strategy": strategy.as_str(),
            "groupFingerprint": CodexInstallationIdDeriver::group_fingerprint(&group_key),
            "matchesCurrentGroupDerivation": matches_current_group,
        }))
    }
}
