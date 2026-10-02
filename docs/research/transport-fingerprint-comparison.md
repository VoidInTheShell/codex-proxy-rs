# 传输层指纹对比实测报告：协议、行为模型、TLS 指纹（多 agent 核实修正版）

> 2026-10-01 实测，同日经 4 个独立 agent 交叉核实修正（解析器复核、官方 0.159.x 源码核实、行为表逐项核实、生产抓包进程归因）。
> 方法：ClientHello 探针（官方 CLI 0.159.2 本地）+ 生产 tcpdump 归因抓包（网桥侧 pre-NAT 源 IP）+ 双方 Cargo.lock/源码比对 + tshark 三方验证。

## 一、TLS 指纹实测结果（核实修正后）

| 路径 | 栈 | cipher | SNI | ALPN | 与官方一致性 |
|---|---|---|---|---|---|
| 官方 codex 0.159.2 HTTP 路径（Linux） | native-tls → OpenSSL | 30 | 目标域名（IP 字面量时无，属正常） | 无 | 基线 |
| **codex-proxy-rs HTTP 路径**（API 调用、自更新、npm/oaistatic） | **vendored OpenSSL（native-tls，刻意固定）** | 30 | **chatgpt.com / api.github.com / registry.npmjs.org / persistent.oaistatic.com**（实测全读出） | 无 | ✅ **cipher-hash 与官方逐位一致（`8d44cdc55eec`）**——Cargo.toml 注释明示"固定 Linux 发布版 TLS 画像，与官方 Codex Desktop 一致" |
| **codex-proxy-rs WS 路径**（socks5 隧道内） | rustls 0.23.45 + aws-lc-rs（同官方 ExplicitCodexTls） | 10 | **chatgpt.com**（隧道内透传） | 无 | ✅ 与官方 WS 路径同栈同 fork；扩展顺序逐连接随机化（与官方同款反指纹特性） |

**核实裁决（推翻此前三处误判）**：

1. ~~"SNI 缺失，P0 修复"~~ → **撤回**。系我的解析器 bug（`sni_of` 把 2 字节大端 name_len 的高位单字节当长度，恒读 0）。修正后所有生产 ClientHello 的 SNI 均正常（chatgpt.com 等）。JA4 第 3 字符 `d` 早已自证 SNI 存在。
2. ~~"OpenSSL 指纹来自 warp/其他进程"~~ → **错误归因**。网桥侧抓包证明全部 30-cipher 连接的源容器就是 codex-proxy-rs（172.21.0.5）。代理是**有意的双 TLS 栈设计**：HTTP=native-tls（OpenSSL 画像）、WS=rustls（官方 WS 同栈），与官方 0.159.x 的双栈行为精确对应。aether-app（rustls+boringssl，无 OpenSSL）、bunkerweb、tailscaled 均排除。
3. ~~"代理全路径 rustls，HTTP 辅助端点指纹与官方不符"~~ → **不成立**，该加固项取消。

**唯一残留的传输层差异（源码级，非实测差异）**：代理 rustls 启用 `prefer-post-quantum`（groups 含 X25519MLKEM768 双 key share），官方 0.159.x 依赖图未启用（单 X25519 key share，MLKEM 在 groups 末位）。仅 WS 路径可见；如需逐字节对齐，关闭代理的 PQ 偏好即可（一行配置级改动）。

## 二、依赖栈与协议栈对比（核实后）

| 依赖 | 官方 codex（0.159.x = main） | codex-proxy-rs 3.18.3 | 一致性 |
|---|---|---|---|
| reqwest | 0.12.28（default features → native-tls 默认） | 0.12.28（同） | ✅ |
| HTTP TLS 后端 | native-tls 默认（`TlsBackend::TransportDefault`；rustls 仅 custom CA/协商失败回退） | native-tls（vendored OpenSSL 固定画像；rustls 仅 custom CA 回退） | ✅ 双栈分工一致 |
| WS TLS 后端 | rustls（`WebSocketTlsMode::ExplicitCodexTls` + aws-lc-rs provider） | rustls 同版本同 provider（`ensure_rustls_provider` 与官方 `codex-utils-rustls-provider` 同构） | ✅ |
| tokio-tungstenite | openai-oss-forks rev `0e5b2d73` | **同一 rev** | ✅ 逐字节 |
| tungstenite | openai-oss-forks rev `4fffad30`（deflate+proxy） | 同 | ✅ |
| hyper / h2 | 1.8.1 / 0.4.19 | 1.11.1 / 0.4.16 | ⚠️ 版本差，但双方上游均无 ALPN → 走 HTTP/1.1，h2 SETTINGS 指纹不适用，风险解除 |
| rustls PQ | 未启用 prefer-post-quantum | 启用 | ⚠️ WS 路径唯一残留差异 |
| WS 握手头 | 13 头集合与顺序（session-id/thread-id 相邻） | `headers.rs:299-330` 锁定 | ✅ 核实逐头一致 |
| zstd level3 / 强制 stream | HTTP SSE 路径 | 同（仅 HTTP SSE 路径；WS 帧为未压缩 Text + permessage-deflate） | ✅ |
| turn-metadata `\uXXXX` 转义 | — | `request.rs:620-651`（普通 to_string 触发上游 Close 1000） | ✅ 核实 |

## 三、行为模型对比（核实修正后，全部经 file:line 证实）

| 行为维度 | 官方客户端（基线） | codex-proxy-rs 现状 | 证据 |
|---|---|---|---|
| OAuth 刷新时点 | exp 前 5min 按需、8 天兜底、单账号单机；**401=立即终态** | **30s 扫描周期**（workers.rs:19）、提前 **3600s**（migration 0001:79）、并发 2（:80）、**401 有界退避**（5 次指数→10min 恢复→2h 上界，token_client.rs:643-673，注释自认与官方语义相反） | 证实 |
| usage 查询 | 客户端 UI 驱动、低频、随用户在线 | 30s worker 只拉**到期账号**（新号补快照/耗尽号 30min 复核）；正常账号靠**响应被动同步**；订阅/资料为管理端**按需**调用（无周期遍历）——比此前描述温和，但集中式低频轮询特征仍在 | 修正后 |
| 请求节奏 | 人类分钟级间歇、会话内串行 | `request_interval_ms=50` 最小间隔（0001:82）+ smart 摊平 → 持续中高并发、无昼夜节律 | 证实 |
| 会话拓扑 | 一账号=一设备=一 session，单并发 | 同账号多下游并发 session（部署配置 max_concurrent=5；**代码默认 3**，0017 仅放宽约束）；**单请求最多 32 次跨账号重放**（routing/mod.rs:27）+ 128KB/2.5s 提交前缓冲（provider/mod.rs:127,130）；逃逸换号时清 turn state/previous_response_id（request.rs:459-487，证实） | 证实+修正 |
| WS 连接生命周期 | 每客户端会话一条、随会话关闭 | 池化 max_age 55min、ping 25s/超时 30s（pool/mod.rs:36-41），单连接跨下游复用 | 证实 |
| 版本分布 | 用户长尾升级 | versionMode=latest 全池同步（appcast/npm 24h 轮询，profile/mod.rs:35） | 证实 |
| 模型目录 | 启动拉取一次 | 15min 周期 + ETag 事件驱动 daemon（workers.rs:293-316） | 证实 |
| UA 画像 | 真实设备环境（terminal 真实值） | 代码默认 **Codex Desktop**（profile/mod.rs:84-101；本部署配置为 codex-tui/macOS）；**terminal 恒 `unknown`**；全局共享+client key 覆盖 | 证实+修正 |
| residency | 仅受管账号发送 | **条件发送**（配置 us 才发；代码默认 None 不发）——本部署配置了 us | 修正 |
| 遥测存在性 | statsig OTLP + analytics 默认开启 | **零遥测** | 证实 |
| warmup | 无 | 北京时间整点对全池发固定 "hello" body（quota/mod.rs:1035-1215；**默认关闭**，本部署未启用） | 证实 |
| 429 处理 | 展示限额、停歇 | 冷却 60s（Retry-After 优先，quota/mod.rs:63）+ 换号继续 | 证实 |

## 四、修正后的暴露特征清单（按风险排序）

### 致命级（部署实测命中，与降级强相关）

1. **多账号同出口 IP**：kiramux39 + yuanqi1031 共用 208.214.166.102；ruby + beihai3body 共用 45.129.9.128（日本机房 ASN，cf-ray NRT 实测）
2. **数据中心 ASN 直连**：2 账号从 VPS IP 直连 chatgpt.com（社区实测降级/封禁头号信号；PRO 账号降级实锤即发生在 ISP 代理出口的持续共享使用后）
3. **UA 画像与出口地理矛盾 + terminal 恒 unknown**：全局唯一 `codex-tui/0.159.3 (Mac OS 15.7.1; arm64) unknown` 出现在美国 ISP/日本机房多出口

### 高危（架构行为差异，全部源码证实）

4. OAuth 批量节奏：30s 扫描 + 提前 1h + 401 退避（官方语义相反）——auth.openai.com 侧可见"单 IP 维护多账号 token 生命周期"
5. 会话拓扑：同账号多并发 session、亲和逃逸秒级换号（清 turn state）、单请求 32 次跨账号重放（同 session-id 数秒内携不同 account-id 依次出现）
6. 请求节律：50ms 最小间隔 + smart 摊平 + 无昼夜节律（真实用户间歇单并发、有作息）
7. 版本齐步走（latest 全池同步升级）

### 中危

8. 零遥测静默（"有推理请求但从未上报遥测的 installation"）
9. usage/订阅/目录/appcast 的集中式周期访问（虽比此前描述温和，仍与真实单用户节奏不符）
10. WS 池 55min 长连接跨用户复用
11. rustls PQ 差异（WS 路径 groups/key share 与官方不同，传输层唯一残留项）

### 传输层已对齐、无需改动（核实确认）

SNI ✅、ALPN ✅、HTTP TLS 画像 ✅（vendored OpenSSL 固定官方 hash）、WS TLS 栈 ✅（同 fork 同 rev）、WS 头序 ✅、zstd/stream ✅、installation-id 语义 ✅（官方经 body client_metadata，代理同路径）、reqwest/rustls/tungstenite 版本 ✅

## 五、降级与恢复结论（不变，经复核属实）

- PRO 账号（kiramux39）请求 gpt-6.1-sol → 上游谎报 responseModel=sol，行为指纹 **99.79% 命中 gpt-5.6-luna**（lmfpd 原始 JSON 复核属实；`x-codex-safety-buffering-faster-model: gpt-5.6-luna` 响应头佐证）
- PLUS 账号 terra 正常（98.46% 命中自身）——terra 低于降级线或触发条件未满足
- 恢复路径按证据强度：换住宅/原生出口 IP → 静置衰减（10min~数天）→ 避免网页版交叉触发 → 降并发去规律化运营；turn-state 注入类对抗工具保质期极短，不推荐

## 六、修正后加固清单

| 优先级 | 项目 | 状态变化 |
|---|---|---|
| **P0** | 每账号独立出口 IP（拆 kiramux39/yuanqi1031 共享代理；ruby/beihai3body 摘除机房直连） | 维持 |
| **P0** | free 号移出 PLUS 组（166 次 no_available_provider 失败噪声） | 维持 |
| ~~P0~~ | ~~SNI 缺失修复~~ | **撤销（误报）** |
| ~~P1~~ | ~~TLS 后端按路径对齐~~ | **撤销（本已对齐）** |
| **P1** | OAuth 刷新去批量化（按需+随机抖动、401 改官方一次性终态语义） | 维持（token_client.rs:643 / workers.rs:19 / refresh.rs:233） |
| **P1** | 重放上限 32→2-3（routing/mod.rs:27）、逃逸换号改排队、max_concurrent 5→2、smart→sticky | 维持 |
| **P1** | terminal 池化真实值 + 版本长尾滞留（latest→落后 1-2 版分布） | 维持 |
| **P2** | rustls PQ 偏好关闭（与官方 WS 指纹逐字节对齐） | 新增（低成本） |
| **P2** | 遥测仿真心跳、lmfpd 式指纹自检探针（降级自动摘号） | 维持 |

## 七、运维备忘（核实过程沉淀）

- SSH 连 JPGREEN 必带 `-o IPQoS=cs0`（DSCP QoS 标记被中间链路掐 session，即此前 Broken pipe 根因）
- 依赖升级须按 providers/openai/Cargo.toml 注释要求重采样 ClientHello 画像（vendored OpenSSL 画像是有意固定的兼容契约）
- 生产容器重启 10s 内恢复 healthy，测试期重启无副作用
- 复核证据留存（服务器）：/tmp/verify_cap*.pcap、/tmp/ss_snap*.txt、/tmp/lmfpd_*.json
