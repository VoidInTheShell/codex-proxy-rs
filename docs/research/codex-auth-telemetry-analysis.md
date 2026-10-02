# OpenAI Codex CLI（codex-rs）鉴权与遥测机制深度分析报告

> 分析对象：`github.com/openai/codex` 主分支源码（codex-rs/ Rust 实现，commit `7b88d09d`，2026-10-01）
> 分析目标：梳理所有**可被 OpenAI 服务端用于判定"订阅共享/账号滥用"的客户端行为特征**
> 本机已安装版本：codex-cli 0.159.2

---

## 1. OAuth 鉴权链路

### 结论
Codex CLI 使用 auth.openai.com 作为 OAuth issuer，`client_id = app_EMoamEEZ73f0CkXaXp7hrann`，浏览器 PKCE 授权码流为主、设备码流为辅。ChatGPT 模式下 access_token 有效期内主动刷新窗口为**过期前 5 分钟**，兜底刷新周期为 **8 天**。refresh_token 为**一次性轮换**（rotation），服务端可检测"refresh token 被复用"（refresh_token_reused）——这是检测同一凭据在多处并发使用的**服务端原生机能**。

### 证据

**Issuer 与 client_id**：
```rust
// login/src/server.rs:77
pub(super) const DEFAULT_ISSUER: &str = "https://auth.openai.com";

// login/src/auth/manager.rs:1718
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
```

**浏览器 PKCE 登录**（本地回调服务器，默认端口 1455，备用 1457）：
```rust
// login/src/server.rs:193-198
const DEFAULT_PORT: u16 = 1455;
const FALLBACK_PORT: u16 = 1457;
let redirect_uri = format!("http://127.0.0.1:{actual_port}/auth/callback");
```

授权 URL 构造（携带 originator 给登录服务器）：
```rust
// login/src/server.rs:594-616
let mut extra_parameters = vec![
    ("id_token_add_organizations", "true"),
    ("codex_cli_simplified_flow", "true"),
    ("originator", originator.as_str()),
];
build_authorization_url(AuthorizationRequest {
    endpoint: &format!("{issuer}/oauth/authorize"),
    client_id, redirect_uri,
    scope: Some("openid profile email offline_access api.connectors.read api.connectors.invoke"),
    ...
```

**授权码换 token**（form 编码）：
```rust
// login/src/oauth/client.rs:44-56  (encoding: TokenEncoding::Form)
let mut parameters = vec![
    ("grant_type", "authorization_code"),
    ("client_id", self.endpoint.client_id),
    ("code", grant.code),
    ("redirect_uri", grant.redirect_uri),
    ("code_verifier", grant.pkce.code_verifier.as_str()),
];
```

登录成功后还会做一次 **token-exchange 换取 API key**（存入 auth.json 的 OPENAI_API_KEY）：
```rust
// login/src/server.rs:1034-1045
"grant_type=urn:ietf:params:oauth:grant-type:token-exchange
 &client_id=...&requested_token=openai-api-key
 &subject_token={id_token}&subject_token_type=urn:ietf:params:oauth:token-type:id_token"
```

**设备码流**（远程/无浏览器场景）：
```rust
// login/src/device_code_auth.rs:56-68
let url = format!("{auth_base_url}/deviceauth/usercode");   // POST {client_id}
// 轮询
let url = format!("{auth_base_url}/deviceauth/token");       // POST {device_auth_id, user_code}
```

**auth.json 完整结构**：
```rust
// login/src/auth/storage.rs:49-73
pub struct AuthDotJson {
    pub auth_mode: Option<AuthMode>,
    #[serde(rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    pub tokens: Option<TokenData>,
    pub last_refresh: Option<DateTime<Utc>>,
    pub agent_identity: Option<AgentIdentityStorage>,
    pub personal_access_token: Option<String>,
    pub bedrock_api_key: Option<BedrockApiKeyAuth>,
    pub bedrock_access_keys: Option<BedrockAccessKeysAuth>,
}

// login/src/token_data.rs:12-33  (tokens 字段)
pub struct TokenData {
    pub id_token: IdTokenInfo,   // 解析后的 JWT claims
    pub access_token: String,    // JWT
    pub refresh_token: String,
    pub account_id: Option<String>,
}
pub struct IdTokenInfo {
    pub email: Option<String>,
    pub chatgpt_plan_type: Option<PlanType>,   // free/plus/pro/business/enterprise/edu
    pub chatgpt_user_id: Option<String>,
    pub chatgpt_account_id: Option<String>,    // workspace id
    pub chatgpt_account_is_fedramp: bool,
    pub raw_jwt: String,
}
```

**Token 刷新**（JSON 编码 POST）：
```rust
// login/src/auth/manager.rs:1591-1600 + oauth/client.rs:57-69 (TokenEncoding::Json)
let mut parameters = vec![
    ("grant_type", "refresh_token"),
    ("client_id", self.endpoint.client_id),
    ("refresh_token", grant.refresh_token),
];
// POST https://auth.openai.com/oauth/token  (manager.rs:212)
```

**刷新阈值**：
```rust
// login/src/auth/manager.rs:203-204
const TOKEN_REFRESH_INTERVAL: i64 = 8;                                  // 天
const CHATGPT_ACCESS_TOKEN_REFRESH_WINDOW_MINUTES: i64 = 5;             // 分钟
// manager.rs:3010-3017: access_token JWT exp <= now+5min 时主动刷新
```

**刷新失败分类**（服务端错误码直接暴露复用检测）：
```rust
// login/src/auth/manager.rs:1670-1693
Some("refresh_token_expired")     => RefreshTokenFailedReason::Expired,
Some("refresh_token_reused")     => RefreshTokenFailedReason::Exhausted,
Some("refresh_token_invalidated")=> RefreshTokenFailedReason::Revoked,
```

**auth mode 差异**：`AuthMode` 枚举含 `Chatgpt / ChatgptAuthTokens / ApiKey / BedrockApiKey / BedrockAccessKeys / Headers / AgentIdentity / PersonalAccessToken`。ChatGPT 模式请求 Codex 后端（chatgpt.com/backend-api/codex），API key 模式走 api.openai.com/v1；`ChatgptAuthTokens`（外部注入的 ChatGPT token，即"共享 token"方案）**只允许内存临时存储**（`storage_mode()` 强制 Ephemeral，manager.rs:1786-1794），且会被遥测标记（见第 4 节 auth_env）。

### 对"订阅共享检测"的影响
- **refresh_token 一次性轮换是服务端最强的共享检测点**：多台机器共用同一 auth.json 时，第二次使用旧 refresh_token 会触发 `refresh_token_reused`，服务端可直接封禁。
- 同一账号在不同机器并发刷新，auth.openai.com 可看到同一账号多个 IP/UA 并行获取 token。
- `ChatgptAuthTokens` 外部注入模式在客户端协议层就被视为特殊路径，遥测会上报其环境特征。

---

## 2. 对 chatgpt.com/backend-api/codex 的出站请求特征

### 结论
Codex 后端默认 base URL 为 `https://chatgpt.com/backend-api/codex`。所有请求自动携带 `originator`、格式化 `User-Agent`（含 OS 版本/架构/终端类型）、`Authorization: Bearer`、`ChatGPT-Account-Id`。`/responses` 请求额外携带 session-id/thread-id 等十余个标识 header。**`purpose=stmt`、`intent` 等 query 参数在当前 Rust 版本中不存在**（属旧 Node.js CLI 遗留）。

### 证据

**Base URL**：
```rust
// model-provider-info/src/lib.rs:77
pub const CHATGPT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
```

**默认 headers（所有请求）**：
```rust
// login/src/auth/default_client.rs:455-469
pub fn default_headers() -> HeaderMap {
    headers.insert("originator", originator().header_value);
    headers.insert(USER_AGENT, get_codex_user_agent());
    // 可选 residency header（us）
}

// login/src/auth/default_client.rs:152-165  User-Agent 格式
let prefix = format!(
    "{}/{build_version} ({} {}; {}) {}",
    originator.value.as_str(),     // codex_cli_rs
    os_info.os_type(), os_info.version(),     // Ubuntu 24.04
    os_info.architecture().unwrap_or("unknown"), // x86_64
    user_agent()                   // 终端类型 token，来自 TERM_PROGRAM 等
);
// 实际形如: codex_cli_rs/0.59.0 (Ubuntu 24.04; x86_64; tmux/3.4)
```

**originator 取值枚举**：
```rust
// login/src/auth/default_client.rs:49,141-149
pub const DEFAULT_ORIGINATOR: &str = "codex_cli_rs";
pub fn is_first_party_originator(v: &str) -> bool {
    v == DEFAULT_ORIGINATOR || v == "codex-tui" || v == "codex_vscode" || v.starts_with("Codex ")
}
pub fn is_first_party_chat_originator(v: &str) -> bool {
    v == "codex_atlas" || v == "codex_chatgpt_desktop"
}
// 可被 CODEX_INTERNAL_ORIGINATOR_OVERRIDE 环境变量覆盖（default_client.rs:50）
```

**chatgpt-account-id 获取与发送**：来自 id_token 的 `chatgpt_account_id` claim（token_data.rs:44），每个请求经 BearerAuthProvider 注入：
```rust
// model-provider/src/bearer_auth_provider.rs:34-43
if let Some(account_id) = self.account_id.as_ref() {
    let _ = headers.insert("ChatGPT-Account-ID", header);
}
if self.is_fedramp_account {
    let _ = headers.insert("X-OpenAI-Fedramp", HeaderValue::from_static("true"));
}
```

**/responses 请求 headers**：
```rust
// codex-api/src/requests/headers.rs:5-13
insert_header(&mut headers, "session-id", &id);
insert_header(&mut headers, "thread-id", &id);
// codex-api/src/endpoint/responses.rs:88-93
insert_header(&mut headers, "x-client-request-id", thread_id);   // = thread_id
insert_header(&mut headers, "x-openai-subagent", &subagent);     // review/compact/memory_consolidation/collab_spawn
```

其余 header 常量（core/src/client.rs:160-180）：
```rust
pub const OPENAI_BETA_HEADER: &str = "OpenAI-Beta";                    // websocket 握手: responses_websockets=2026-02-06
pub const X_CODEX_INSTALLATION_ID_HEADER: &str = "x-codex-installation-id";
pub const X_CODEX_ROUTING_HINT_HEADER: &str = "x-codex-routing-hint";
pub const X_CODEX_TURN_STATE_HEADER: &str = "x-codex-turn-state";     // 服务端下发的粘性路由 token，轮次内回传
pub const X_CODEX_TURN_METADATA_HEADER: &str = "x-codex-turn-metadata"; // JSON 大块元数据(见§5)
pub const X_CODEX_PARENT_THREAD_ID_HEADER: &str = "x-codex-parent-thread-id";
pub const X_CODEX_WINDOW_ID_HEADER: &str = "x-codex-window-id";       // 压缩窗口 id
pub const X_OPENAI_SUBAGENT_HEADER: &str = "x-openai-subagent";
pub const X_OAI_ATTESTATION_HEADER: &str = "x-oai-attestation";       // 宿主(App)提供的设备证明 token
pub const X_RESPONSESAPI_INCLUDE_TIMING_METRICS_HEADER: &str = "x-responsesapi-include-timing-metrics";
const X_OPENAI_INTERNAL_CODEX_RESPONSES_LITE_HEADER: &str = "x-openai-internal-codex-responses-lite";
// 另有 "x-codex-beta-features"（实验特性清单，session/mod.rs:1216-1231）
```

**session_id / conversation_id / thread_id 生成规则**：
```rust
// protocol/src/session_id.rs:19-24
pub fn new() -> Self { Self { uuid: Uuid::now_v7() } }   // UUIDv7（时间有序）
```
- `session_id`：每会话一个 UUIDv7（进程内）
- `thread_id`（即历史中的 conversation_id）：每会话/线程一个 UUIDv7
- `window_id`：每压缩窗口一个
- **跨请求复用**：同一会话所有 /responses 请求复用同一 session-id/thread-id；`prompt_cache_key = session_id`（core/src/client.rs:581-593），服务端可据此观察缓存亲和与请求连续性

**请求 URL 清单**（backend-client/src/client.rs 及各文件，PathStyle::ChatGptApi 前缀 `/wham`，CodexApi 前缀 `/api/codex`）：

| 端点 | 方法 | 位置 |
|---|---|---|
| `{base}/responses` | POST (SSE/ws) | codex-api/src/endpoint/responses.rs:96 |
| `/wham/usage`（rate limits+plan+credits） | GET | rate_limit_resets.rs:127 |
| `/wham/accounts/check` | GET | client.rs:402 |
| `/wham/profiles/me`（token usage profile） | GET | client.rs:419 |
| `/wham/settings/user` | GET | client.rs:547 |
| `/wham/workspace-messages` | GET | client.rs:729 |
| `/wham/config/bundle`（云端配置下发） | GET | client.rs:526 |
| `/wham/rate-limit-reset-credits`(+`/consume`) | GET/POST | rate_limit_resets.rs:134-151 |
| `/wham/usage/thread_usage/query`(/`query_v2`) | POST | thread_usage.rs:115 / task_usage.rs:99 |
| `/wham/usage/thread-estimates/query` | POST | chatgpt_turn_cost.rs:52 |
| `api.chatgpt.com/v1/analytics/codex/turn-costs` | POST | turn_usage.rs:62 |
| `/wham/usage/daily-token-usage-breakdown` 等 7 个 analytics 报表 | GET | client/analytics.rs:44-78 |
| `{chatgpt_base}/codex/analytics-events/events` | POST | analytics/src/client.rs:154 |
| `/wham/app/appcast`（版本检查） | GET | cli/src/doctor/updates.rs:43 |
| `/wham/remote/control/server/{enroll,refresh,pair}` | POST | remote_control/protocol.rs:224-230 |
| `auth.openai.com/oauth/{authorize,token,revoke}`、`/deviceauth/{usercode,token}` | - | login crate |
| `ab.chatgpt.com/otlp/v1/metrics`（Statsig 遥测） | POST | otel/src/config.rs:9 |

`anthropic` header：不存在。`purpose=stmt` 全仓库无结果（旧版 Node CLI 遗留）；`intent` 仅 realtime 语音场景作为 query 参数出现（`intent=quicksilver`，codex-api/src/endpoint/realtime_call.rs:221），常规 /responses 请求不携带。

### 对"订阅共享检测"的影响
- 服务端每请求都能拿到：**originator + 精确 CLI 版本 + OS/架构/终端类型（UA）、installation_id、account_id、session_id、thread_id、window_id**。同一账号下出现多个并发的 session_id 且 installation_id 不同、UA/OS 差异大、地理 IP 不同，是共享订阅的典型服务端信号。
- `x-codex-turn-state`（粘性路由 token）+ `session-id` 可用于检测"同一 session 从不同 IP/网络连续请求"。

---

## 3. 设备指纹与安装标识

### 结论
唯一的客户端持久设备标识是 `installation_id`：`~/.codex/installation_id` 中的 UUIDv4，经 body `client_metadata` 与 `x-codex-turn-metadata`（JSON 大块 header）两条通道随每个 /responses 请求上报；名为 `x-codex-installation-id` 的 HTTP header **仅用于 remote_control websocket**（`app-server-transport/src/transport/remote_control/websocket.rs:70`），常规 /responses 请求不发送。CLI 自身**不采集** machine-id/hwid；`x-oai-attestation` 设备证明 header 由宿主应用（ChatGPT Desktop 等）按需提供，CLI 本体不生成。SQLite 状态库存储大量本地标识（cwd、git remote、creator 身份），但仅本地使用。

> 复核修正（2026-10-01）：原报告称 installation_id 经「header/client_metadata/turn-metadata 三通道上报」不准确——常规请求无 header 通道；下游代理不发送 `x-codex-installation-id` header 与官方行为一致，不构成相对暴露。

### 证据

**installation_id 生成与存储**：
```rust
// core/src/installation_id.rs:17-61
pub(crate) const INSTALLATION_ID_FILENAME: &str = "installation_id";
// 读取 ~/.codex/installation_id，无效/缺失则生成新 UUIDv4 写回
// 注意: options.mode(0o644) —— 权限宽松，任何用户可读
let installation_id = Uuid::new_v4().to_string();
```

**上报路径 1——client_metadata**（core/src/responses_metadata.rs:325-332）：
```rust
let mut client_metadata = HashMap::from([
    (X_CODEX_INSTALLATION_ID_HEADER.to_string(), self.installation_id.clone()),
    (SESSION_ID_KEY.to_string(), self.session_id.clone()),
    (THREAD_ID_KEY.to_string(), self.thread_id.clone()),
    (X_CODEX_WINDOW_ID_HEADER.to_string(), self.window_id.clone()),
]);
```

**上报路径 2——turn metadata JSON 内**（responses_metadata.rs:418）：
```rust
installation_id: has_request_identity.then_some(self.installation_id.as_str()),
```

**`x-codex-installation-id` header 的真实用途**：仅 remote_control websocket 携带（`app-server-transport/src/transport/remote_control/websocket.rs:70`、`remote_control/server_api.rs:30`）；常规 /responses 的 HTTP header 集（`compatibility_headers()` responses_metadata.rs:377-402）只插 window-id/turn-metadata/parent-thread-id/subagent，无 installation-id。

**x-oai-attestation**（不透明设备证明 token，CLI 内置实现返回 None）：
```rust
// app-server-protocol/src/protocol/v2/attestation.rs:15-18
pub struct AttestationGenerateResponse {
    /// Opaque client attestation token.
    pub token: RedactedString,
}
// app-server/src/attestation.rs:30  超时 100ms，仅支持 attestation-capable 连接（宿主 App）
const ATTESTATION_GENERATE_TIMEOUT: Duration = Duration::from_millis(100);
```

**SQLite 状态库**（state/src/sqlite.rs:34-39，均在 ~/.codex/ 下）：
```rust
const LOGS_DB_FILENAME: &str = "logs_2.sqlite";            // 日志(含 thread_id, process_uuid)
const GOALS_DB_FILENAME: &str = "goals_1.sqlite";
const MEMORIES_DB_FILENAME: &str = "memories_1.sqlite";
const QUEUE_DB_FILENAME: &str = "queue_1.sqlite";
const STATE_DB_FILENAME: &str = "state_5.sqlite";          // threads 主表
const THREAD_HISTORY_DB_FILENAME: &str = "thread_history_1.sqlite";
```

`state_5.sqlite` threads 表本地存储（state/migrations/0001_threads.sql + 0053/0056）：
```sql
CREATE TABLE threads (
    id TEXT PRIMARY KEY, rollout_path TEXT, cwd TEXT NOT NULL, title TEXT,
    sandbox_policy TEXT, approval_mode TEXT, tokens_used INTEGER,
    git_sha TEXT, git_branch TEXT, git_origin_url TEXT, ...
);
ALTER TABLE threads ADD COLUMN originator TEXT;
ALTER TABLE threads ADD COLUMN creator_user_id TEXT;
ALTER TABLE threads ADD COLUMN creator_account_id TEXT;
```
这些库**仅本地**（不直接上传），但相同信息会经 turn-metadata/analytics 事件外发。

macOS 另有 `user-verification` crate（Secure Enclave 设备密钥，用于 cyber 验证，见 §7），不在 Linux 上启用。

### 对"订阅共享检测"的影响
- **installation_id 是账号共享检测的核心静态指纹**：同一 account/user 下出现 N 个不同 installation_id，或多台机器共用同一 CODEX_HOME（installation_id 相同但 IP/UA 漂移）都可直接聚类。
- 无法从 CLI 侧伪造 attestation（由宿主生成），但 CLI 本体不带硬件指纹。

---

## 4. 遥测与打点系统（重点）

### 结论
Codex 有**三套独立遥测**：
1. **OTel/Statsig 指标**：默认开启（release 构建），上报至 `https://ab.chatgpt.com/otlp/v1/metrics`，带内置 API key。含 ~100 种 metric（turn 时长/token 用量/成本估算/工具调用/启动阶段…）。核心使用量类 metric 已从 Statsig 通道排除。
2. **Codex analytics 事件流**：默认开启，POST `{chatgpt_base}/codex/analytics-events/events`，携带完整 turn 级事件（token 计数、工具调用数、时间分解、运行时 OS/架构/版本、客户端信息）。API key 模式下仅发插件类事件；ChatGPT 模式全量发送。
3. **OTel log/trace**：默认关闭（exporter=None），需用户显式配置 OTLP；开启后可上报完整用户 prompt 文本（log_user_prompt，默认 false→"[REDACTED]"）。

**无 Sentry/crash reporter**。用户可通过 `[analytics] enabled = false` 关闭 1 和 2。

### 证据

**Statsig OTLP 端点与 API key**（硬编码）：
```rust
// otel/src/config.rs:9-11
pub(crate) const STATSIG_OTLP_HTTP_ENDPOINT: &str = "https://ab.chatgpt.com/otlp/v1/metrics";
pub(crate) const STATSIG_API_KEY_HEADER: &str = "statsig-api-key";
pub(crate) const STATSIG_API_KEY: &str = "client-MkRuleRQBd6qakfnDYqJVR9JuXcY57Ljly3vi5JVUIO";
// resolve_exporter(): debug 构建默认关闭，release 构建启用 (config.rs:14-30)
```

**默认开关**（config/src/types.rs:651-659）：
```rust
impl Default for OtelConfig {
    fn default() -> Self {
        OtelConfig {
            exporter: OtelExporterKind::None,          // 日志导出默认关
            trace_exporter: OtelExporterKind::None,    // 追踪导出默认关
            metrics_exporter: OtelExporterKind::Statsig, // 指标默认→Statsig
```
`analytics_enabled = false`（config.toml `[analytics] enabled=false`）时 metrics 也关（core/src/otel_init.rs:61-67）。TUI/exec 入口 `default_analytics_enabled = true`（tui/src/startup_orchestration.rs:626）。

**被排除出 Statsig 的指标**（只走用户自配 OTLP）：
```rust
// otel/src/metrics/config.rs:23-35
const STATSIG_DISABLED_METRICS: &[&str] = &[
    API_CALL_COUNT_METRIC, API_CALL_DURATION_METRIC, CONVERSATION_TURN_COUNT_METRIC,
    TOOL_CALL_COUNT_METRIC, TOOL_CALL_DURATION_METRIC,
    TURN_COST_MICROUSD_METRIC, TURN_TOKEN_USAGE_METRIC, ...
];
```
指标名样例（otel/src/metrics/names.rs）：`codex.turn.token_usage`、`codex.turn.cost_microusd`、`codex.turn.e2e_duration_ms`、`codex.tool.call`、`codex.sse_event`、`codex.thread.started`、`codex.process.start`、`codex.sqlite.init.count`、`codex.startup.phase.duration_ms`、`codex.guardian.review`、`codex.plugins.*`、`codex.hooks.run`…

**会话遥测元数据**（每次会话/请求打点都带）：
```rust
// otel/src/events/session_telemetry.rs:100-118
pub struct SessionTelemetryMetadata {
    pub conversation_id: ThreadId,
    pub agent_name: String,
    pub auth_mode: Option<String>,        // api_key / chatgpt
    pub auth_env: AuthEnvTelemetryMetadata,
    pub account_id: Option<String>,
    pub user_id: Option<String>,
    pub account_email: Option<String>,
    pub originator: String,
    pub product_sku: Option<&'static str>,
    pub session_source: String,
    pub model: String, pub slug: String,
    pub service_tier: Option<String>,
    pub model_reasoning_effort: Option<String>,
    pub app_version: &'static str,
    pub terminal_type: String,
}
```

**auth_env 环境探测上报**（login/src/auth_env_telemetry.rs:41 —— 直接探测用户是否用环境变量注入凭据）：
```rust
pub struct AuthEnvTelemetryMetadata {
    pub openai_api_key_env_present: bool,
    pub codex_api_key_env_present: bool,
    pub codex_api_key_env_enabled: bool,
    pub provider_env_key_name: Option<String>,
    pub provider_env_key_present: Option<bool>,
    pub refresh_token_url_override_present: bool,   // 是否覆盖了刷新端点(代理!)
}
```

**用户 prompt 日志**（可选，默认脱敏）：
```rust
// otel/src/events/session_telemetry.rs:1148-1176
let prompt_to_log = if self.metadata.log_user_prompts { prompt.as_str() } else { "[REDACTED]" };
log_event!(self, event.name = "codex.user_prompt",
    prompt_length = %prompt.chars().count(), prompt = %prompt_to_log);
```

**Turn 成本/token 打点**（本地估算 + 服务端数据）：
```rust
// session_telemetry.rs:400-421
tags = vec![("turn.id", turn_id), ("conversation.id", conversation_id.as_str()),
            ("turn.interrupted", ...), ("speed", speed), ("reasoning_effort", ...)];
self.counter(TURN_COST_MICROUSD_METRIC, estimated_microusd, &tags);
log_event!(self, event.name = "codex.turn_cost", usage.estimated_usd = estimated_usd, ...);
```

**analytics 事件流目标与发送条件**：
```rust
// analytics/src/client.rs:154,314-324
url: format!("{base_url}/codex/analytics-events/events"),   // base = chatgpt_base_url
pub fn new(auth_manager, base_url, analytics_enabled: Option<bool>) -> Self {
    let destination = ...;
    Self { queue: (analytics_enabled != Some(false)).then(...) }   // 默认开启
}
// client.rs:946-951: API key 模式过滤
if auth.is_api_key_auth() { events.retain(TrackEventRequest::can_send_with_api_key_auth); }
else if !auth.uses_codex_backend() { return; }
```

**事件类型全集**（analytics/src/events.rs:70-105）：`thread_initialized`、`turn_event`、`turn_steer`、`command_execution`、`file_change`、`mcp_tool_call`、`dynamic_tool_call`、`control_tool_call`、`collab_agent_tool_call`、`web_search`、`image_generation`、`accepted_line_fingerprints`、`skill_invocation`、`plugin_used/installed/install_requested/install_failed/measurements`、`hook_run`、`compaction`、`goal_*`、`guardian_review`、`app_mentioned/app_used`、`artifact_operation`、`thread_hint_status`、`external_agent_config_import_*`。

**turn_event 载荷字段**（events.rs:1075-1146，节选）：
```rust
pub(crate) struct CodexTurnEventParams {
    thread_id, session_id, turn_id, active_plugin_ids_at_turn_start, root_turn_id,
    turn_trigger, codex_turn_source, submission_type,
    app_server_client: CodexAppServerClientMetadata,   // product_client_id, client_name, client_version, rpc_transport
    runtime: CodexRuntimeMetadata,                     // codex_rs_version, runtime_os, runtime_os_version, runtime_arch
    ephemeral, thread_source, subagent_source, parent_thread_id,
    model, model_provider, sandbox_policy, reasoning_effort, reasoning_summary,
    service_tier, approval_policy, approvals_reviewer, guardian_v2_enabled,
    sandbox_network_access, collaboration_mode, personality, workspace_kind,
    num_input_images, is_first_turn, status, usage_limit_window_minutes,
    steer_count, total_tool_call_count, shell_command_count, file_change_count,
    mcp_tool_call_count, subagent_tool_call_count, web_search_count, image_generation_count,
    input_tokens, cached_input_tokens, cache_write_input_tokens, output_tokens,
    reasoning_output_tokens, total_tokens,
    before_first_sampling_ms, sampling_ms, compaction_ms, tool_blocking_ms, ...,
    sampling_request_count, sampling_retry_count, duration_ms, started_at, completed_at,
}
```

**接受的代码行指纹事件**（旧行为保留字段）：
```rust
// analytics/src/events.rs:206-219
pub(crate) struct CodexAcceptedLineFingerprintsEventParams {
    turn_id, thread_id, product_surface, model_slug, completed_at,
    repo_hash: Option<String>,
    accepted_added_lines: u64, accepted_deleted_lines: u64,
    line_fingerprints: [(); 0],   // 已停用，schema 兼容保留
}
```

**SQLite 健康遥测**（state/src/lib.rs:137-155）：`codex.sqlite.corruption.count`、`codex.sqlite.init.count/duration_ms`、`codex.sqlite.logs.write.*`、`codex.sqlite.fallback.count`。

**Sentry / crash reporting**：全仓库无 sentry 依赖，无 panic 上报模块。

### 对"订阅共享检测"的影响
- **analytics 事件流 = 服务端视角的完整行为画像**：每 turn 的 token 用量、工具调用数、时长分解、模型/effort、甚至 OS/架构/CLI 版本、终端类型，全部直接进入 chatgpt.com 后端。共享订阅场景下，服务端能看到同一账号**异常的 token 消耗速率、并发 thread 数、迥异的 runtime 环境**。
- `auth_env` 遥测会暴露"环境变量注入 API key/覆盖刷新端点"等**代理/共享工具特征**。
- `[analytics] enabled=false` 可以关掉这两条通道（但 /responses 请求上的 turn-metadata 仍在）。

---

## 5. Responses API 请求体中的环境泄露

### 结论
`store=false, stream=true`——**每个 turn 请求都完整重发全部会话历史**（无 previous_response_id 链）。请求体 `client_metadata` 中嵌入 `x-codex-turn-metadata` JSON：包含 **git 仓库根路径、git remote URL（脱敏用户名/密码）、HEAD commit hash、是否有未提交改动、完整工具清单、沙箱模式**等。用户消息前注入 `<environment_context>`，包含 **绝对 cwd 路径、shell、当前日期/时区**；realtime 场景还会附**目录树**。`prompt_cache_key = session_id`。

### 证据

**请求体结构**（codex-api/src/common.rs:279-311 + core/src/client.rs:996-1008）：
```rust
pub struct ResponsesApiRequest {
    pub model: String,
    pub stream: bool,                      // 恒 true
    pub service_tier: Option<String>,
    pub instructions: String,              // 基础系统提示词
    pub input: Vec<ResponseItem>,          // 完整会话历史 items（每轮全量重发）
    pub tools: Option<ResponsesApiTools>,
    pub tool_choice: String,               // "auto"
    pub parallel_tool_calls: bool,
    pub reasoning: Option<Reasoning>,
    pub store: bool,                       // 恒 false
    pub stream_options: Option<StreamOptions>,
    pub include: Vec<String>,              // ["reasoning.encrypted_content"]
    pub prompt_cache_key: Option<String>,  // = session_id
    pub text: Option<TextControls>,
    pub client_metadata: Option<HashMap<String, String>>,
    pub access_programs: Option<AccessPrograms>,
}
// core/src/client.rs:1003-1004: store: false, stream: true
// core/src/client.rs:581-593: prompt_cache_key = session_id（或 "{source}:{parent_thread_id}"）
```

**turn-metadata JSON 载荷**（core/src/responses_metadata.rs:515-585，CodexTurnMetadataPayload 序列化字段）：
```rust
installation_id, session_id, thread_id, agent_name, turn_id, window_id, window_number,
context_window_id, request_kind(turn/prewarm/compaction/memory), forked_from_thread_id,
forked_from_ordinal_exclusive, parent_thread_id, parent_turn_id, root_turn_id,
subagent_kind, thread_source, turn_trigger, sandbox, sandbox_mode,
auto_review_enabled, node_repl_*, workspaces, tool_namespaces_info,
turn_started_at_unix_ms, history_ingest_requested, analytics_enabled, compaction,
extra  // 含 model, reasoning_effort, source, workspace_kind 等
```

**git 仓库信息注入**（core/src/responses_metadata.rs:198-204 + core/src/turn_metadata.rs:530-562）：
```rust
pub(crate) struct TurnMetadataWorkspace {
    pub associated_remote_urls: Option<BTreeMap<String, SanitizedGitUrl>>,  // 全部 remote URL（去凭证）
    pub latest_git_commit_hash: Option<String>,   // HEAD SHA
    pub has_changes: Option<bool>,                // 是否有未提交改动
}
// workspaces 以 {仓库根绝对路径: {...}} 形式序列化
// SanitizedGitUrl (protocol/src/sanitized_git_url.rs:72-74): 去除用户名/密码，但保留 host+path
```

**environment_context（用户消息内）**（core/src/context/world_state/environment.rs:322-346）：
```rust
fn push_environment_values(...) {
    rendered.push_str("<cwd>");   // 绝对路径
    rendered.push_str("<shell>"); // /bin/bash 等
}
// 另含 <current_date>, <timezone>, <network .../>, <filesystem .../>, <subagents>
```

**目录树泄露（realtime/语音场景）**（core/src/realtime_context.rs:364-398）：
```rust
let mut lines = vec![
    format!("Current working directory: {}", cwd_path.display()),
    format!("Working directory name: {}", file_name_string(cwd_path)),
];
if let Some(git_root) = &git_root {
    lines.push(format!("Git root: {}", git_root.display()));
    lines.push(format!("Git project: {}", file_name_string(git_root)));
}
// "Working directory tree:" + 文件树（TREE_MAX_DEPTH 层、DIR_ENTRY_LIMIT 项）
```

`include_environment_context` 默认 true（core/src/config/mod.rs:4029），可配置关闭。

**会话历史传递**：因 `store=false`，模型侧无服务端会话状态；每轮请求 `input` 数组携带**从会话开始以来的全部 items**（含 reasoning encrypted content 回传）。跨会话内容不自动带入（fork 场景带 forked_from_thread_id 元数据）。

**MCP 请求也带 turn 元数据**（core/src/turn_metadata.rs:293-317）：外部 MCP 服务器会收到含 `codex_version`、user_input_requested_during_turn 的元数据副本。

### 对"订阅共享检测"的影响
- turn-metadata 中的 **git remote URL + commit hash + 仓库路径**构成强关联特征：服务端可直接观察"同一账号在不同机器上操作相同/不同仓库"，或多个账号集中操作同一私有仓库（团队共享凭据的典型特征）。
- 全量历史重发使服务端能完整重建每次会话内容（请求级审计能力）。
- cwd 绝对路径泄露用户名/主机名（`/home/<user>/...`），可辅助多设备聚类。

---

## 6. 并发与限流行为

### 结论
- 请求重试：最多 4 次（默认），200ms 基准指数退避 ±10% jitter；**HTTP 429 不重试**（retry_429=false），5xx/传输错误重试；遵守 Retry-After。
- 限流状态来自**响应头**（x-codex-primary/secondary-*）与 `/wham/usage` 端点轮询（由 app-server 的 account/rateLimits RPC 驱动，TUI 后台定时拉取）。
- primary/secondary 双窗口由服务端定义，客户端仅展示。
- 429→UsageLimitReached 错误带 plan_type/resets_at/window/promo；**客户端无"活跃状态上报"**——活跃信号由请求流量本身+analytics turn 事件构成。
- 401 时客户端做**一次** token 刷新后重试。

### 证据

```rust
// model-provider-info/src/lib.rs:64-65,448-454
const DEFAULT_STREAM_MAX_RETRIES: u64 = 5;
const DEFAULT_REQUEST_MAX_RETRIES: u64 = 4;
let retry = ApiRetryConfig {
    max_attempts: self.request_max_retries(),
    base_delay: Duration::from_millis(200),
    retry_429: false,        // 429 不自动重试
    retry_5xx: true,
    retry_transport: true,
};

// codex-client/src/retry.rs:46-57,60-70
fn should_retry(...) {
    (self.retry_429 && status.as_u16() == 429) || (self.retry_5xx && status.is_server_error())
}
pub fn backoff(base, attempt) -> Duration {
    let jitter: f64 = rand::rng().random_range(0.9..1.1);   // ±10% 抖动
}
// 遵守 Retry-After: err.retry_after() 优先于退避
```

**限流响应头解析**（codex-api/src/rate_limits.rs:63-80）：
```rust
let prefix = format!("x-{normalized_limit}");  // 默认 x-codex，多 limit: x-{limit_id}-*
"primary-used-percent" / "primary-window-minutes" / "primary-reset-at"
"secondary-used-percent" / "secondary-window-minutes" / "secondary-reset-at"
```

**/wham/usage 轮询**（app-server/src/request_processors/account_processor.rs:1190-1224）：
```rust
async fn get_account_rate_limits_response(...) {
    ...
    if params.supports_luna_reserve && auth_mode == Chatgpt && !fedramp {
        client.get_rate_limits_with_luna_reserve().await   // 附 x-openai-codex-luna-reserve: 1
    } else {
        client.get_rate_limits_with_reset_credits().await
    }
}
```

**UsageLimitReached 语义**（protocol/src/error.rs:683-689）：
```rust
pub struct UsageLimitReachedError {
    pub plan_type: Option<PlanType>,
    pub resets_at: Option<DateTime<Utc>>,
    pub limit_window_minutes: Option<u16>,       // 服务端判定哪个窗口触发
    pub rate_limits: Option<Box<RateLimitSnapshot>>,
    pub promo_message: Option<String>,
    pub rate_limit_reached_type: Option<RateLimitReachedType>,  // 含 workspace credits/spend cap 类型
}
```

401 恢复（core/src/client.rs:2528-2546）：单次 ChatGPT token 刷新重试（PendingUnauthorizedRetry），带 auth_recovery 遥测事件。

无 primary/secondary 的客户端区分逻辑（纯服务端语义），无心跳/keepalive 上报。

### 对"订阅共享检测"的影响
- 429 不重试 + Retry-After 遵守，意味着客户端不会用重试风暴暴露自己；但**频繁触发 429 本身**（服务端视角：同账号短窗高频请求）即是最直接的滥用信号。
- rate-limit 轮询是低频后台 GET，对检测贡献小。
- 多设备共享时，服务端在 usage 维度（5h/weekly 窗口用量）最先看到异常。

---

## 7. LUNA 相关

> 复核补充（2026-10-01）：safety-buffering 机制为原报告盲点，补记如下。
>
> **safety-buffering（安全缓冲处理）**：服务端通过响应头 `x-codex-safety-buffering-enabled` 与 `x-codex-safety-buffering-faster-model` 声明 treatment（`codex-api/src/safety_buffering.rs:4-20`）；任一 header 存在即构造 `SafetyBufferingTreatment{faster_model}`，enabled 的值不 gate faster_model（专门测试 `buffering_enabled_header_does_not_gate_the_faster_model_fallback` 确认）。SSE 事件流内亦可携带 `safety_buffering` 元数据（use_cases 如 `["cyber"]`、reasons 如 `["user_risk"]`、retry_model；`sse/responses.rs:243-265`）；合并规则：wire 显式 retry_model 用 wire 值，缺失回退 HTTP header，显式 null 不回退。客户端消费链**纯通知**：TUI 展示「正在为这个请求多加思考」提示与手动重试选项（用 faster_model 重试整轮，`tui/src/app/safety_buffering.rs`），主响应流不中断不丢弃，faster_model **绝不自动替换主响应**。与 ModelReroute（flagged 重路由，服务端已换模型）是两条独立链路。对降级归因的含义：safety-buffering 头单独出现不构成降级证据（社区干净抓包中未降级响应同样携带），但 use_cases/reasons 揭示账号处于何种风控评估链路；不排除服务端在风控链路中直接用 faster_model 完成主响应——此为服务端行为，客户端源码无法证实或证伪。

### 结论
Luna 是 OpenAI 的模型代号（gpt-5.6-luna / gpt-6-luna），同时衍生出 **"Luna Reserve"** 机制：普通用量耗尽后允许后端切换到 Luna 保留容量（实验性）。客户端通过 `x-openai-codex-luna-reserve: 1` header 在 usage 查询时声明支持，**该声明同时授权后端记录实验曝光（experiment exposure）**。客户端无本地降级逻辑；**账号被标记（flagged）时由服务端将请求重路由到 gpt-5.2**，客户端仅识别并展示警告（cyber-safety reroute）。

### 证据

**模型 ID**：
```rust
// model-provider-info/src/lib.rs:86-92
pub const AMAZON_BEDROCK_GPT_6_LUNA_MODEL_ID: &str = "openai.gpt-6-luna";
pub const AMAZON_BEDROCK_GPT_5_6_LUNA_MODEL_ID: &str = "openai.gpt-5.6-luna";
pub const AMAZON_BEDROCK_RUNTIME_GLOBAL_GPT_5_6_LUNA_MODEL_ID: &str = "global.openai.gpt-5.6-luna";
// model-provider/src/amazon_bedrock/catalog.rs:18,21
const GPT_5_6_LUNA_OPENAI_MODEL_ID: &str = "gpt-5.6-luna";
const GPT_6_LUNA_OPENAI_MODEL_ID: &str = "gpt-6-luna";
// model-provider/src/provider.rs:126,130
const API_KEY_APPROVAL_REVIEW_PREFERRED_MODEL: &str = "gpt-5.6-luna";
pub const DEFAULT_MEMORY_EXTRACTION_PREFERRED_MODEL: &str = "gpt-5.6-luna";
```

**Luna Reserve（限流备用容量实验）**：
```rust
// backend-client/src/client/rate_limit_resets.rs:29-30,73-77
/// Opt in only for clients that can apply Reserve, not for passive account usage readers.
pub async fn get_rate_limits_with_luna_reserve(&self) -> Result<RateLimitsWithResetCredits> {
    self.get_rate_limits_for_usage(/*supports_luna_reserve*/ true).await
}
if supports_luna_reserve {
    req = req.header("x-openai-codex-luna-reserve", HeaderValue::from_static("1"));
}

// app-server-protocol/src/protocol/v2/account.rs:313-319
/// The client supports automatic Luna Reserve fallback. For eligible ChatGPT CLI users,
/// allow the backend to record experiment exposure after ordinary usage is blocked.
pub supports_luna_reserve: bool,
```

**Reserve 回退文案**（protocol/src/error.rs:691-697）：
```rust
// Reserve is a fallback for exhausted ordinary usage, so keep the standard
// promo/plan recovery copy below instead of suggesting another model.
if limit_name ... != "gpt-reserve" { ... }
```

**flagged（cyber-safety 重路由）——服务端主动降级，客户端检测模型不一致**：
```rust
// core/src/session/mod.rs:3979-4010
async fn maybe_warn_on_server_model_mismatch(..., server_model: String) -> bool {
    ...
    warn!("server reported model {server_model} while requested model was {requested_model}");
    let warning_message = format!(
        "Your account was flagged for potentially high-risk cyber activity and this request \
         was routed to gpt-5.2 as a fallback. To regain access to gpt-5.3-codex, apply for \
         trusted access: {CYBER_VERIFY_URL} or learn more: {CYBER_SAFETY_URL}"
    );
    self.send_event(turn_context, EventMsg::ModelReroute(ModelRerouteEvent {
        from_model, to_model, reason: ModelRerouteReason::HighRiskCyberActivity,
    }))
}
// core/src/session/mod.rs:507-508
const CYBER_VERIFY_URL: &str = "https://chatgpt.com/cyber";
const CYBER_SAFETY_URL: &str = "https://developers.openai.com/codex/concepts/cyber-safety";
```

相关：macOS 有 `user-verification` crate（Secure Enclave 密钥 + 证明），对应 cyber 验证的客户端凭据侧（user-verification/src/lib.rs:1，非 macOS 返回 unsupported）。

**downgrade/degraded 其他出现**：均为 Arc::downgrade（弱引用）无关项；无其它网络降级逻辑。

### 对"订阅共享检测"的影响
- flagged→gpt-5.2 重路由是**服务端风控的直接动作**（针对高风险活动而非共享本身），但同一逻辑体系可用于任意服务端降级；客户端只透传结果。
- Luna Reserve 的 usage 查询附带"允许记录实验曝光"信号，服务端可借此做 A/B 分层，滥用检测策略可能按该维度分层。

---

## 8. 版本与更新机制

### 结论
CLI 通过 User-Agent（`originator/{version}`）、OTel service_version、analytics 事件 `codex_rs_version` 三处上报版本。启动时（默认开启，可关）后台 GET GitHub latest release 或 Homebrew cask API 检查新版本，结果缓存 `~/.codex/version.json`。**无强制更新/版本拒绝机制**——旧版本可持续工作（服务端当然可自行按版本差别对待）。`originator` 枚举见 §2。

### 证据

```rust
// tui/src/updates.rs:27-34,61-62  启动检查（仅提示，不强制）
pub fn get_upgrade_version(config: &Config) -> Option<String> {
    if !config.check_for_update_on_startup || is_source_build_version(CODEX_CLI_VERSION) { ... }
}
const HOMEBREW_CASK_API_URL: &str = "https://formulae.brew.sh/api/cask/codex.json";
const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/openai/codex/releases/latest";
// 结果写 version.json 缓存（updates_cache.rs）

// cli/src/doctor/updates.rs:36-43  doctor 检查
const GITHUB_LATEST_RELEASE_URL: &str = "https://api.github.com/repos/openai/codex/releases/latest";
const DESKTOP_UPDATE_URL: &str = "https://persistent.oaistatic.com/codex-app-prod/appcast.xml";
const BACKEND_DESKTOP_UPDATE_URL: &str = "https://chatgpt.com/backend-api/wham/app/appcast";

// core/src/config/mod.rs:1137-1140,4083  默认开启可关闭
/// Defaults to `true`.
pub check_for_update_on_startup: bool;
let check_for_update_on_startup = cfg.check_for_update_on_startup.unwrap_or(true);
```

版本字符串来源：`env!("CARGO_PKG_VERSION")`（analytics events.rs CodexRuntimeMetadata.codex_rs_version；User-Agent get_codex_user_agent）。

无版本拒绝逻辑（无最低版本检查/kill switch）；服务端可按 header 中的版本自行分流。

---

## 客户端可被服务端观测的全部信号清单

| # | 信号 | 载体 | 生成/存储位置 | 共享检测价值 |
|---|---|---|---|---|
| 1 | `installation_id`（UUIDv4，机器级持久） | header `x-codex-installation-id`、client_metadata、turn-metadata | `~/.codex/installation_id`（0644） | ★★★ 每机器唯一 ID，同账号多 ID = 多设备 |
| 2 | `session_id`（UUIDv7，每会话） | header `session-id`、client_metadata、`prompt_cache_key` | 进程内存 | ★★★ 并发会话数/会话连续性 |
| 3 | `thread_id`（UUIDv7，=conversation_id） | header `thread-id`、`x-client-request-id`、analytics 事件 | 进程内存 + sqlite | ★★ 线程拓扑（fork/parent） |
| 4 | `window_id` / `context_window_id` | header / turn-metadata | 进程内存 | ★ 压缩行为特征 |
| 5 | `chatgpt-account-id` | header `ChatGPT-Account-Id` | id_token claim | ★★★ workspace 归属 |
| 6 | `chatgpt_user_id` / email / plan_type | id_token、遥测元数据 | auth.json | ★★★ 账号身份 |
| 7 | `originator`（codex_cli_rs/codex-tui/codex_vscode/codex_atlas/…） | header + authorize URL | env 覆盖 | ★★ 客户端类型 |
| 8 | 完整 `User-Agent`：`{originator}/{版本} ({OS} {版本}; {arch}; {终端})` | 所有 HTTP 请求 | os_info + TERM_PROGRAM | ★★★ 版本+OS+终端指纹 |
| 9 | `access_token`（JWT，含 exp/iss） | Authorization: Bearer | auth.json / 内存 | ★★ token 生命周期 |
| 10 | `refresh_token` 使用序列（一次性轮换） | POST auth.openai.com/oauth/token | auth.json | ★★★ 多处复用 → refresh_token_reused |
| 11 | Token 刷新时序（exp-5min / 8 天兜底） | auth.openai.com 日志 | - | ★★ 刷新频率异常 = 多客户端并发 |
| 12 | `x-oai-attestation`（设备证明，App 宿主） | header（仅 attestation-capable 客户端） | 宿主提供 | ★★（CLI 本体不生成） |
| 13 | `x-codex-turn-state`（服务端粘性路由 token 回传） | header | 服务端下发 | ★★ 同 turn 跨源回放检测 |
| 14 | turn-metadata JSON：`sandbox`、`sandbox_mode`、`approval_policy` | body client_metadata | 运行时配置 | ★★ 配置异常聚类 |
| 15 | turn-metadata `workspaces`：**git remote URL（脱敏）、HEAD SHA、has_changes、仓库根绝对路径** | body client_metadata | git 命令采集 | ★★★ 仓库/代码指纹 |
| 16 | turn-metadata `tool_namespaces_info`（完整工具/MCP 清单） | body client_metadata | 运行时 | ★★ 插件/工具画像 |
| 17 | `<environment_context>`：**绝对 cwd、shell、时区、当前日期、网络状态、文件系统权限** | body input（用户消息） | 运行时 | ★★★ 主机路径/用户名泄露 |
| 18 | 全量会话历史 items（store=false 每轮重发） | body input | - | ★★★ 服务端可完整审计内容 |
| 19 | `model`、`reasoning_effort`、`service_tier`、`speed` | body + header | 配置 | ★★ 用量画像 |
| 20 | `x-codex-beta-features`（实验特性清单） | header | 配置 | ★ |
| 21 | OTel/Statsig 指标（~100 种：turn 时长/token/成本/工具调用/启动/DB…） | POST ab.chatgpt.com/otlp/v1/metrics | 默认开（release） | ★★★ 行为画像（可关） |
| 22 | analytics 事件流（thread_initialized/turn_event/…，含全部 token 计数与时间分解） | POST {chatgpt_base}/codex/analytics-events/events | 默认开 | ★★★ 最完整行为画像（可关） |
| 23 | `codex_rs_version`、`runtime_os/os_version/arch`（analytics RuntimeMetadata） | analytics 事件 | - | ★★★ 环境指纹 |
| 24 | `product_client_id`、`client_name`、`client_version`、`rpc_transport`（app-server 客户端信息） | analytics 事件 | - | ★★ |
| 25 | `auth_mode`（api_key/chatgpt）+ `auth_env`（**环境变量注入检测、刷新端点覆盖检测**） | OTel 元数据 | env 探测 | ★★★ 代理/共享工具特征 |
| 26 | `terminal_type`（TERM_PROGRAM 派生） | OTel 元数据 | env | ★ |
| 27 | `account_email`、`user_id` | OTel 元数据 | id_token | ★★ |
| 28 | 429/限流响应头消费行为（x-codex-primary/secondary-*） | header（被动） | - | ★★ 触发频率即信号 |
| 29 | `/wham/usage`、`/wham/accounts/check`、`/wham/profiles/me` 等周期性 GET | 请求日志 | app-server 后台轮询 | ★★ 启动/轮询指纹 |
| 30 | `x-openai-codex-luna-reserve: 1`（Luna Reserve 实验曝光授权） | header | 支持 Luna 的客户端 | ★ 实验分层 |
| 31 | `accepted_line_fingerprints` 事件（repo_hash、接受行数统计） | analytics 事件 | git diff | ★★ 代码贡献画像 |
| 32 | CLI 版本（三通道：UA/OTel/analytics） | 见 #8/#21/#23 | CARGO_PKG_VERSION | ★★★ |
| 33 | 请求源 IP / TLS 指纹 / HTTP2 指纹（reqwest 默认栈） | 网络层 | - | ★★★（服务端侧，非客户端代码控制） |
| 34 | OAuth 登录流参数（scope、codex_cli_simplified_flow、originator） | authorize URL | - | ★ |
| 35 | `OpenAI-Beta: responses_websockets=2026-02-06`（ws 握手） | header | 常量 | ★ 传输方式画像 |
| 36 | 重试模式（429 不重试、5xx 4 次、±10% jitter） | 请求序列 | - | ★★ 自动化工具检测基线 |

### 关闭开关汇总
- `[analytics] enabled = false`（config.toml）→ 关闭 Statsig 指标 + analytics 事件流（core/src/otel_init.rs:61-67、analytics/src/client.rs:321）
- `[otel] exporter/trace_exporter/metrics_exporter = "none"` → 各通道独立控制（默认仅 metrics=Statsig）
- `include_environment_context = false` → 关闭 cwd/shell 注入（config/src/profile_toml.rs:57）
- **无法关闭**：/responses 请求上的 session/thread/installation 标识 headers、client_metadata、turn-metadata（协议必需），以及网络层指纹。
