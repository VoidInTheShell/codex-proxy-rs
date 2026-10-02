# codex-proxy-rs 订阅共享风控暴露面与 LUNA 降级实测报告

> 调研日期：2026-10-01（同日经 4 路独立复核修正 + 传输层归因复核五轮）。本文档为当前状态的实证记录：JPGREEN 生产部署实测 + 官方 codex 源码分析 + 社区证据交叉。
> 关联报告：[codex 官方鉴权与遥测分析](codex-auth-telemetry-analysis.md)、[OpenAI 风控与模型指纹调研](openai-risk-control-and-model-fingerprint-report.md)、[传输层指纹对比（修正版）](transport-fingerprint-comparison.md)
>
> 复核修正记录（2026-10-01 四路独立复核 + 生产归因复核）：
> - 降级结论升级：PRO 账号指纹判定经独立特征线复核（数值分布/末位分布/开头元组签名三条线全部命中 luna，gpt-6.1-sol 在 53 模型排名中仅列第 7、概率差 2.6 万倍），离线重跑 bit 级一致；另发现社区独立 HAR 级实证（community 1399076：官方端点直连请求 astra、响应体三处 model ID 均为 luna、额度仅 4%）
> - 修正 A4：`x-codex-safety-buffering-*` 头在两个 plus 账号上均出现且 terra 指纹为真，krasnik 独立抓包同样显示未降级响应携带该头——头单独出现不构成降级证据，仅为服务端安全缓冲处理的告知信号
> - 修正 B4：30s 是 worker 唤醒周期而非上游轮询频率；/usage 每账号稳态 ≥30 分钟，subscriptions/profiles/me 为管理端按需调用，非周期任务
> - 修正 B5：32 为路由 attempt 总上限（含同账号瞬态/普通重试四类），非「跨账号重放」次数；传输恢复另有独立预算
> - 修正 B11：官方常规 /responses 请求同样不发送 `x-codex-installation-id` header（仅 remote_control WS 使用），网关行为与官方一致，不构成暴露；installation_id 实际经 body client_metadata + turn-metadata 两通道
> - 修正 D1：「机房 IP 基本必进降级通道」表述过强——ruby 从日本机房 IP 直连仍正常服务 terra，pwinner 完美住宅环境仍被降级；IP 质量是评分因子之一，既非充分也非必要条件
> - 修正 D3：网页版触发降级为单源观察（pwinner），无独立复现
> - 修正 D4：「312 信号一天失效」指检测信号存续期（约 20.5h，实锤），非注入技术存续期（state TTL≈1h，社区声称有效性带作者免责）
> - 传输层归因终审：此前误判「生产 ClientHello 无 SNI」与「OpenSSL 指纹来自 warp/其他进程」均被推翻——SNI 全部正常（解析器 bug）；OpenSSL 30-cipher 指纹就是 codex-proxy-rs 自己的 HTTP 路径（有意 vendored OpenSSL 固定官方画像，与官方 Linux codex 逐位一致）；代理为有意双 TLS 栈设计（HTTP=native-tls、WS=rustls），与官方 0.159.x 双栈行为精确对应，传输层指纹已对齐无需改动（唯一残留：WS 路径 rustls prefer-post-quantum 与官方不同，P2 项）

## 一、JPGREEN 部署现状实测

- 部署形态：Docker Compose（compose 标签 3.15.2，容器内实际二进制为本地构建的 3.18.3），bunkerweb 反代，仅监听 `127.0.0.1:8080`
- 账号池：4 个 OAuth 账号
  - `kiramux39@gmail.com`（pro，经美国 ISP socks5 代理 208.214.166.102 出口，华盛顿州）
  - `yuanqi1031@gmail.com`（plus，同一美国 ISP 代理出口）
  - `ruby.gender.64+youtube@icloud.com`（plus，服务器直连出口 45.129.9.128，日本数据中心 IP）
  - `beihai3body@gmail.com`（free，直连出口；对非免费模型一律 400 unsupported，是当前 no_available_provider 报错主因）
- 运行参数：`rotation_strategy=smart`、`max_concurrent_per_account=5`、`request_interval_ms=50`、`refresh_margin_seconds=3600`、`refresh_concurrency=2`、warmup 关闭、`request_location_enabled=false`、residency=us
- 请求画像：openai provider 走 CLI/macos/tui 预设，`versionMode=latest`（当前 0.159.3），全实例共享同一 UA
- 下游用户：codex-tui 0.153.4~0.159.2（Windows/Debian）+ Orca + pi，经分组 key（PLUS/PRO/PC）接入

## 二、LUNA 降级实测结论（lmfpd 行为指纹）

用 lmfpd（lm.ikale.io 同源 CLI）经网关对本机 `127.0.0.1:8080/v1` 采样，参考库 53 模型（GPT-5.6 全家为官方直连采样）：

| 测试 | 命中账号 | 请求模型 | 指纹判定 | 概率 |
| --- | --- | --- | --- | --- |
| PRO 组 | kiramux39（pro，美国 ISP 出口） | gpt-6.1-sol | **gpt-5.6-luna** | 0.9979（ranker+verifier 一致） |
| PLUS 组 | yuanqi1031 + ruby（同轮混合） | gpt-5.6-terra | gpt-5.6-terra（正常） | 0.9846 |

要点：

- PRO 账号被降级实锤：请求 gpt-6.1-sol，响应 SSE 元数据谎报 `responseModel=gpt-6.1-sol`，但行为指纹以 99.79% 概率命中 gpt-5.6-luna。复核确认：完整 53 模型排名中 gpt-6.1-sol 仅列第 7（概率 3.8e-05，与 luna 差约 2.6 万倍），luna 一骑绝尘；独立特征线（开头元组签名：探针三样本开头恰为 luna 参考库模态开头，合计命中 13/36；6.1-sol 模态开头命中 0/36）交叉验证
- 对照组（terra）经同一网关链路正确命中自身参考，证明网关链路不扭曲指纹——阴性对照成立
- 社区独立 HAR 级实证（community 1399076，2026-09-19~23）：官方端点直连请求 gpt-6-astra，响应体 response.created/in_progress/completed 三处 model ID 均为 gpt-5.6-luna，额度仅用 4%（排除 Luna Reserve 兜底），turn-state 312 字符；OpenAI 支持人员已向用户确认 response.created 的 model 字段即实际服务模型
- `x-codex-safety-buffering-enabled/faster-model` 头在两个 plus 账号（含真 terra 响应）上均出现：该头是服务端安全缓冲处理的告知信号（客户端仅展示「额外思考」提示、faster_model 为手动重试候选，绝不自动替换主响应），单独出现不构成降级证据；但其 `use_cases: ["cyber"]` 语义表明账号处于 cyber 风控评估链路
- 上游响应携带 `x-codex-turn-state` 加密票据；社区证据（cockpit-tools/ccodex-sleep-state）表明其长度与风控档位相关（292/312/332 模式，该检测信号存续约 20.5 小时即失效，OpenAI 会快速轮换）
- 方法论保留：gpt-6.1-sol 参考样本来自 openrouter/openai（Chat Completions）渠道而非官方直连（5.6 家族为 openai/direct）；探针为正向命中 luna，该缺陷不动摇结论但降低区分精度

## 三、上游请求抓包实录（request_dump 证据）

网关上游 WebSocket 握手头（按发送顺序）：`Host: chatgpt.com`、`Connection: Upgrade`、`Upgrade: websocket`、`Sec-WebSocket-Version: 13`、`Sec-WebSocket-Key`、`version: 0.159.3`、`x-client-request-id: req_*`、`x-codex-routing-hint: model=*`、`openai-beta: responses_websockets=2026-02-06`、`originator: codex-tui`、`user-agent: codex-tui/0.159.3 (Mac OS 15.7.1; arm64) unknown (codex-tui; 0.159.3)`、`Authorization`、`chatgpt-account-id`、`x-openai-internal-codex-residency: us`、`cookie: __cf_bm=...`、`sec-websocket-extensions: permessage-deflate; client_max_window_bits`

缺失项（对照官方 CLI 抓包）：

- `session-id` / `thread-id` header（官方 codex-api 每次 /responses 都带；本请求体未含 client_metadata 中的 session_id/thread_id，仅 installation_id）
- `x-codex-installation-id` header（官方三通道上报之一，仅出现在 body 的 client_metadata）
- 账号级 cookie 完备性：仅 `__cf_bm` 一项，官方浏览器态会话会带更多 Cloudflare cookie

已核实的正向特征：installation_id 按账号稳定（跨请求同账号同值）；账号切换时 `chatgpt-account-id`、installation_id、cookie 正确联动；请求体兼容改写（store=false、input 归一化）与官方一致。

## 四、暴露特征清单（按风险排序）

结合源码审计（见代理审计矩阵）、官方源码信号清单与部署实测，当前部署对 OpenAI 服务端可见的共享特征：

### 致命级（部署实测直接命中）

1. **多账号同出口 IP**：kiramux39 + yuanqi1031 共用 208.214.166.102；ruby + beihai3body 共用 45.129.9.128（日本机房 ASN）。同一 IP 维持多账号 token 生命周期/usage 轮询/推理请求
2. **数据中心 ASN 直连**：ruby、beihai3body 从 VPS IP 直连 chatgpt.com（cf-ray NRT 出口实测确认）；机房 IP 是社区实测的降级/封禁高发信号
3. **UA 画像与出口矛盾**：全局唯一 UA `codex-tui/0.159.3 (Mac OS 15.7.1; arm64)`，但出口 IP 在美国 ISP/日本机房间切换，同一「安装」出现在多个网络
4. **terminal 恒为 `unknown`**：官方 CLI 的 terminal 字段总是真实值（TERM_PROGRAM 派生），全池恒 unknown 是统计异常

### 高危（架构性行为差异）

5. OAuth 批量刷新：单 IP 每 30s 扫描周期（仅刷到期账号）、默认提前 3600s（官方为 exp 前 300s，提前 12 倍）、并发 2；401 走有界退避重试——官方语义为 401 即终态（分歧仅限 401 分支；refresh_token_expired/reused/invalidated 三错误码双方语义一致）
6. usage 主动观测：worker 每 30s 唤醒（DB 扫描），上游 /usage 拉取每账号稳态 ≥30 分钟；初始入库突发（每轮最多 100 账号）、额度失败后 2s 补查、catalog 15min ETag 刷新为主要密集点；subscriptions/profiles/me 为管理端打开账号详情时按需单账号调用
7. 跨账号重放：单请求路由 attempt 上限 32（含同账号瞬态/普通重试/换号重试四类，传输恢复另有 4 次建连预算）；提交前缓冲 128KB/2.5s 内换号重放时，同 session-id 在数秒内携带不同 account-id 依次出现
8. 同账号并发会话形态：max_concurrent_per_account=5 + smart 摊平负载，账号呈现持续多并发、全天候活跃，与真实用户间歇单并发曲线完全不同
9. 中途换号：亲和逃逸时同一 session-id 的下一轮请求 account-id 突变、turn state 消失（逃逸后 CAS 重绑新账号）
10. 版本齐步走：versionMode=latest 使全池在同一 UA 版本上同步升级，官方用户是长尾分布（现有 VersionMode::Fixed 可直接用于版本钉住）

### 中危（协议细节）

11. `session-id`/`thread-id` header 缺失：lmfpd 等极简客户端经网关转发的请求头里没有这两个官方必带 header（网关只透传不合成；真实 codex CLI 下游会带上；纯第三方客户端缺失）
12. 遥测静默：官方客户端默认上报 statsig OTLP + analytics 事件流；网关身份零遥测，「有推理请求但永无遥测的安装」可被服务端关联
13. 模型目录/版本元数据后台轮询（15min ETag 刷新 + 每日 appcast/npm），同一身份永不休眠
14. residency=us 全局固定：官方仅受管账号才发送该头
15. WS 连接池跨下游用户复用（55min 寿命、同账号同会话锚复用连接）
16. 网关不发送 `x-openai-codex-luna-reserve` 头（未加入 Luna Reserve 实验曝光）——与官方保守客户端行为兼容，但意味着额度耗尽后无 Reserve 兑底

### 已排除的伪暴露（复核修正）

- `x-codex-installation-id` header 未发送：官方常规 /responses 请求同样不发送该 header（仅 remote_control WS 使用）；installation_id 经 body client_metadata + turn-metadata 两通道，网关与官方一致，不构成暴露

### 降级触发面（本部署实测 + 社区证据）

- PRO 账号（美国 ISP 出口）已处于降级态：gpt-6.1-sol → luna。该账号的使用特征：smart 调度下持续中高并发、池化共享、UA 画像恒定
- 日本机房直连的 ruby 未降级（terra 指纹为真），说明机房 IP 并非降级的充分条件；两个 plus 账号的响应均携带 safety-buffering 头（cyber 风控评估链路在位）
- 社区证据指向降级信号叠加评分：出口 IP 质量（机房/共享为不利因子，但非充分必要）、多账号同 IP、token 集中刷新、行为形态异常；静置恢复（10min~2 天多尺度、多用户复现）支持时间衰减机制；「每窗口首次请求正常」为单源观察

## 五、降级账号恢复路径（按证据强度，经复核重评）

1. **静置衰减（高置信）**：关停该账号请求一段时间（10min~2 天多尺度、多用户复现）后风控分衰减；具体时长与账号状态相关
2. **换干净出口（中高置信，弱形式）**：IP 质量是风控评分因子之一——换住宅/原生出口降低不利信号，但**不构成充分条件**（pwinner 完美住宅环境仍被降级；ruby 机房 IP 正常服务 terra）；本代理已支持每账号 outbound proxy
3. **恢复性运营（推断）**：降低并发至单并发、请求模式去规律化、UA/terminal 画像真实化后再逐步放量；叠加消除多账号同 IP、集中刷新等信号
4. **避免网页版交叉触发（低置信，单源）**：V2EX 单一用户观察「打开 chatgpt.com 网页后下一请求立刻变 luna」，无独立复现，作为运营注意项保留
5. **对抗性手段（不推荐依赖）**：x-codex-turn-state 采集注入——社区声称短期对部分账号有效（state TTL≈1h），但工具作者自带免责与失败案例；「312 检测信号约 20.5 小时失效」证明 OpenAI 对社区信号有快速反制能力

## 六、验证方式复现

- 指纹检测：`npx lmfpd@latest -b http://127.0.0.1:8080/v1 -k <key> -m <model> --json`（服务器本机执行，key 不出服务器）；建议 `-n 3` 多轮增强置信度
- 上游抓包：config.yaml `host.logging.request_dump: true`（含凭据，排障后必须关闭；retention 1 天）
- 账号归因：`model_requests` 表按时间戳对齐采样请求，`provider_account_email_snapshot` 字段；单组多账号时务必逐样本归因（本轮 PLUS 测试即出现同轮跨账号混合采样）
- turn-state 观察：request_dump 的 upstream.response.headers 中 `x-codex-turn-state` 长度（292/312/332 模式；该信号已多次轮换，仅作诊断参考）
- 降级根因区分：指纹命中 luna + 额度未耗尽 + 无 subagent 标记 = 风控降级；safety-buffering 头单独出现不构成降级证据
