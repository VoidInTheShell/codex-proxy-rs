# OpenAI 订阅共享风控、账号降级（LUNA 路由）与模型指纹检测工具调研报告

调研时间：2026-10-01（同日经独立事实核查复核修正）。所有事实标注【实锤】（有 HAR 抓包/官方回应/源码/多源交叉验证）、【传闻】（社区单源说法）、【推测】（基于证据的推断）。
Reddit 因本机网络被墙无法直接读取全文，相关内容依据搜索引擎摘要与其它平台转述，已尽量交叉验证。

> **复核修正记录（2026-10-01 独立核查）**：
> 1. §4.3「VPS 出口基本进降级通道【实锤】」标签越界——多源一致的仅是「IP 质量参与评分」方向；「基本必进」被 pwinner（完美住宅仍降级）、easy88866（美国家宽 1/5 luna）、ruby（机房 IP 正常服务 terra）三重反证，降为【方向性因子】
> 2. §3.5 官方修复出处应为 1391809 #14（非 1391818 #14）
> 3. juice 960→128/372k→272k 事件日期为 **2026-07-14**（多家媒体报道 URL 日期），非 9 月
> 4. 时间线「09-11 发现 312 信号」起点无法验证（社区工具首发均在 9-18），已修正
> 5. 新增强化证据：community 1399076（9-19~23）——官方端点直连 HAR 级 astra→luna 实证（响应体三处 model ID 均 luna、额度仅 4%、turn-state 312 字符）；OpenAI 支持向 Pro 用户确认 response.created 的 model 字段即实际服务模型（见 §3.2 阶段三补充）
> 6. 指纹工具局限补证：ModelTrace #25（sol medium 测出 claude）、#13/#26（GPT-6 家族指纹缺失时间线）；lm-detector 参考库中 gpt-6.1-sol 样本来自 openrouter/openai（非官方直连）
> 7. D3（网页版触发降级）确认为单源（pwinner #15），降为传闻级

---

## 1. lm.ikale.io 模型指纹工具（Fingerpoint Detector / lm-detector / lmfpd）

### 1.1 它是什么

- 网页版：https://lm.ikale.io （标题"模型指纹检测 · Fingerpoint Detector by Ikaleio"），GitHub 仓库 https://github.com/Ikaleio/lm-detector ，CLI 发布为 npm 包 `lmfpd`，致谢 [xqy2006/ModelTrace](https://xqy2006.github.io/ModelTrace/) 提供算法思路与部分原始数据。【实锤】
- 用途：中转站/第三方 API 声称提供某模型，实际可能接入另一个模型；该工具用一个与语义无关的任务核验这一点。【实锤】（README）

### 1.2 它给 LLM 发什么 prompt（实测摘录）

每轮生成 3 道"挑战"，每道要求模型**凭第一反应逐个写出约 300 个 1–355 之间的整数**，要求数量从 292–332 中不重复抽取，语言在中/英/日/韩/法五种中独立随机，措辞由多组等价模板随机组合。提示词明确：禁止调用任何工具（Python、代码执行器、计算器、搜索、API、外部随机数生成器）、禁止先写代码；禁止从 1 开始计数、连续递增/递减、等差、循环、重复区块等规则化模式；允许重复值；直接从第一个取值开始输出。【实锤】（页面实测 + README）

实测页面上三道样本分别为：法语（要 326 个整数）、中文（要 315 个）、韩语（要 320 个）。例（中文样本节选）：

> "生成一组不承载语义的整数选择。逐项选择 315 个 1 到 355（含端点）的整数。每个位置都要单独选择；不要从 1 开始计数，不要连续递增或递减，也不要采用等差、循环、重复区块或其他规则化模式。本任务必须由当前语言模型直接完成：禁止调用或借助任何工具……直接从第一个取值开始输出……"

**原理**：LLM 写出的"随机数"并不均匀——每个模型对数值区间、末位数字的偏好相对稳定，构成行为指纹。【实锤】（README 原理章节）

### 1.3 如何区分不同底层模型（算法流水线）

1. **解析**：取回答中最长一段 1–355 整数序列；有效整数少于 max(80, ⌈0.55N⌉) 的回答剔除（拒答/截断自动排除）。
2. **特征**：① 数值分布——355 个取值计数平滑后开平方（Hellinger 嵌入）；② 位置分段（序列 4 等分 × 16 个取值区间）+ 末位数字 0–9 分布，共 74 维；两块按 0.75:0.25 加权拼接。
3. **排名器**：s = 0.5·z_LDA + 0.25·z_kNN + 0.25·z_去干扰中心（SVD 估计提示环境偏移的 2 维干扰子空间并投影消除）。
4. **核验器**：仅三条回答都有效时运行；对每个候选计算"同一模型/其他模型/库外模型"三假设下的高斯对数密度、似然增益、近邻距离、领先幅度，线性模型合成核验分数；排名第一与核验最高不一致时标注分歧。
5. **置信度**：参考库内温度 softmax（`fpd retrain` 用嵌套留出法拟合 τ，二元 NLL 须低于基线且 AUC>0.75 才会替换检测器）。【实锤】（README 原理章节 + 流程图）

### 1.4 参考库覆盖（实测数据）

- 网页样本库显示：**53 个模型、1,948 条样本**，覆盖 GPT、Claude、Gemini、Grok、Qwen、DeepSeek、GLM、Kimi、MiMo 等。【实锤】
- 直接拉取 `data/unified_bank.json`（构建于 2026-09-30）核验，GPT 家族条目包括：gpt-4o、gpt-5.4、gpt-5.5、**gpt-5.6-luna / gpt-5.6-sol / gpt-5.6-terra、gpt-6-astra / gpt-6-luna / gpt-6-sol、gpt-6.1-sol**，每个 36 条样本、约 1.06–1.21 万个有效数字。**GPT-5.6 三兄弟全部采样自 `openai/direct`（官方 API 直连）**，即参考指纹是"官方正版 Luna/Sol/Terra"的行为基线。【实锤】
- 采样渠道还包括 openrouter、oaipro、mono 等，以及订阅渠道（如 `kimi-code-subscription`）；CLI 支持 `--subscription codex-subscription`，**可直接用本机 Codex 登录（ChatGPT 订阅后端）采样**。【实锤】（README + bank 数据）

### 1.5 对"GPT-5.6-LUNA 是降级路由模型"能提供什么证据

该工具能提供的证据形式：**对一个声称在服务 Sol/Astra 的渠道（Codex 订阅、拼车号池、中转站）跑指纹检测，若稳定命中 gpt-5.6-luna 指纹且置信度高，即为"请求 A 模型实际返回 B 模型"的行为学证据。**

- 社区已有同源做法：V2EX 用户 pwinner 用同算法源头的 [ModelTrace](https://xqy2006.github.io/ModelTrace/) 确认其 Pro 20x 的 astra/sol/terra/5.5 全部路由到 luna，随后 MITM 抓包实锤（见 §3.2）。【实锤】
- linux.do 2026-09-09 有帖子《【Codex降智测评0909】抽检13家高级推广，2家Juice指纹为Luna》，用"糖果题、Juice、包体特征、延迟"四指标抽检 13 家共享/中转商家，其中 2 家指纹为 Luna——说明共享号池账号确实存在被路由到 Luna 的情况。【实锤】（帖子存在与结论；正文因 linux.do 反爬仅获摘要，链接 https://linux.do/t/topic/2877508 ）
- **局限性（工具作者自己强调）**：检测结果只是"参考库内的封闭集合排序"，库外模型也会得到"最像"候选，排名分数与置信度都不是身份证明；判断渠道是否可信需固定参数、重复多轮并结合其他证据。降级若是"同模型减推理预算"（见 §3.2 juice 值）而非换模型，指纹不会变。【实锤】（README IMPORTANT 框）

### 1.6 API 模式怎么用（能否 curl 拿回指纹结论）

- **网页 API 模式**：填 Base URL、API Key、模型名、协议（Responses / Chat Completions / Messages），页面自动采样；请求经同源 `/api/proxy` 转发（上游无需支持 CORS，代理只接受 HTTPS 默认端口的完整域名，密钥不落服务器）。**指纹分析在浏览器端完成，lm.ikale.io 本身没有"提交对话→返回结论"的 REST API**，不能直接 curl 拿结论。【实锤】（README + 实测）
- **实测代理端点**：`curl -X POST https://lm.ikale.io/api/proxy -d '{"url":"https://api.openai.com/v1/chat/completions",...}'` 返回上游式 401（"Supply an API key for the selected service."），证明端点按预期转发。【实锤】（本次实测）
- **等价 curl 方案 = CLI**：`npx lmfpd@latest -b https://api.example.com/v1 -k sk-xxx -m gpt-6-astra --json`，`--json` 向 stdout 输出结构化结论；支持 `--repeat N` 多轮、`--input result.json` 离线重分析、`--output` 保存报告（不含凭据）。本次实测 `npx -y lmfpd@latest --help` 可正常运行（Node≥22 / Bun≥1.4.2）。【实锤】（README + 本次实测）
- 手动模式：只有聊天界面没有 Key 时，复制页面生成的三道挑战发给目标模型，把回答粘回页面，三条有效时给排名+核验分数+置信度。

### 1.7 复现步骤（供后续实测）

```sh
# 1. 检测某个中转/官方渠道（OpenAI 协议）
npx lmfpd@latest -b https://api.example.com/v1 -k sk-xxx -m gpt-5.6-sol --json

# 2. 检测 Codex 订阅渠道（本机 Codex 登录）
npx lmfpd@latest sample --subscription -m gpt-6-astra --subscription-name codex-subscription

# 3. 多轮增强置信度
npx lmfpd@latest -b ... -k ... -m gpt-5.6-sol -n 5 --output result.json
```
判读：若请求 Sol/Astra 稳定命中 gpt-5.6-luna（归因概率显著领先），结合 codex 用量统计（luna 调用占比异常）与 turn-state 长度（§3.4.6）即可形成降级路由的证据链。

---

## 2. ChatGPT/Codex 订阅共享封禁与降级的公开报告

### 2.1 时间线

| 时间 | 事件 | 来源 |
| --- | --- | --- |
| 2023-12～2024-02 | GPT-4 "lazy mode" 抱怨潮，官方承认"非故意"、Altman 称"已不那么懒" | [PCMag](https://www.pcmag.com/news/openai-acknowledges-gpt-4-is-getting-lazy)、[Business Insider](https://www.businessinsider.com/chatgpt-openai-sam-altman-chatbot-less-lazy-after-users-complain-2024-2) |
| 2026-02 | GitHub issue：Codex 显示"账号被标记高风险网络活动，请求被路由到 gpt-5.2 作为回退"，标签 bug + safety-check | [openai/codex#12079](https://github.com/openai/codex/issues/12079) |
| 2026-05 | Codex 登录环节强制手机号验证，社区推断"一号一绑、额度锁定" | [ofox 排查清单](https://blog.csdn.net/ofox_ai_hunter/article/details/161823409) |
| 2026-06-05 | 中文社区（V2EX 等）大量 ChatGPT/Codex 封号帖集中爆发 | [新浪财经](https://finance.sina.com.cn/wm/2026-06-05/doc-iniakfhp1594524.shtml)、[腾讯云社区](https://cloud.tencent.com/developer/article/2688138) |
| 2026-07～08 | GPT-5.6 Pro 请求被 resolved 为 5.5-mini（HAR 实锤，多 Pro 用户） | [community.openai.com 1387941](https://community.openai.com/t/5-6-pro-model-has-been-automatically-downgraded-and-routed-to-the-5-5-mini-model-since-its-release/1387941) |
| 2026-08-20～25 | Plus 账号 Sol→5.5-mini 大规模爆发，官方承认并部署修复 | [1391809](https://community.openai.com/t/chatgpt-web-requests-gpt-5-6-sol-but-server-resolves-gpt-5-5-mini-on-plus-accounts/1391809)、[1391818](https://community.openai.com/t/possible-gpt-5-6-sol-routing-mismatch-selected-gpt-5-6-but-server-metadata-shows-gpt-5-5-mini/1391818) |
| 2026-08-27 | Tibo Sottiaux 确认 Codex 存在"Luna Reserve"额度兜底档 | [Net.Coffee](https://ip.net.coffee/gpt/news/20260828a.html) |
| 2026-09-03 起 | GPT-6 发布后 Pro 20x 账号 "at capacity" → 静默降级到 Luna/4o 级别 | [community 1397173](https://community.openai.com/t/codex-work-gpt-6-astra-has-a-serious-model-downgrading-issue/1397173) |
| 2026-09-18～22 | 社区发现 312 turn-state 风控信号（ccodex-sleep-state 首发 9-18；cockpit-tools v1.3.58 于 9-22 加入判定） | [cockpit-tools releases](https://github.com/jlcodes99/cockpit-tools/releases) |
| 2026-09-23 | 312 信号失效（v1.3.59 移除该功能，"已失效"） | 同上 |
| 2026-09 下旬 | OpenAI 将部分静默降级改为显式阻断（社区报告） | [reddit r/codex 1wnj1xe](https://www.reddit.com/r/codex/comments/1wnj1xe/openai_replaced_silent_model_degradation_with/) |

### 2.2 被封/降级用户的共同行为特征

**封号侧（2026-06 波次，中文社区归纳 + 个案）**：

- 网络：多人共享代理出口（一 IP 后几百上千人，一人违规全段连坐）；30 分钟内跨多国 IP 跳变；已知机房 IP 段（AWS/GCP/DO 被标记为非住宅）；接码平台/批量注册工厂用过的 IP 段。甚至"美国住宅宽带+苹果内购、网络条件良好"也被误伤（IP 段历史关联连坐）。【实锤】（[ofox](https://blog.csdn.net/ofox_ai_hunter/article/details/161823409)、[V2EX 1218155](https://v2ex.com/t/1218155)）
- 支付：虚拟卡/一次性卡；礼品卡（2026 明显收紧）；**同一张卡绑多个账号**（有"用同一张 Wise 卡注册第二个号当晚封号"案例）；卡区与 IP 严重不匹配（土耳其卡+美国 IP）。【实锤】（ofox）
- 账号来源：买号/共享号/代充——原主已被风控关联则连坐；linux.do 案例中被连封 3 个 20x Pro 的账号分别来自 WildAI 开通、咸鱼代充，账单均为菲律宾区支付，封禁邮件理由为"网络滥用"；另有"正价充值但之前搞过漏洞/逆向、掉过订阅的号被封"的归纳。【实锤（个案）】（[linux.do 2521455](https://linux.do/t/topic/2521455)、[2094965](https://linux.do/t/topic/2094965?page=3)、[2693215](https://linux.do/t/topic/2693215)）
- 使用模式：脚本 24h 高 QPS；请求间隔过于规整（精确 2.0s）；headless 浏览器/纯 HTTP 无 cookie/UA/时区指纹；同一订阅被部署到多机同时跑（IP 跳变+高并发双重命中）；API key 多 IP 分发使用。【实锤】（ofox 归纳 + V2EX 案例交叉）
- 重置卡滥用：community 论坛用户提到"不断给用户发重置的人是 Thibault/Tibo（官方），系统允许用却被惩罚的是用户"——重置卡高频兑换被并入风控。【传闻】（[community 1397173 #21](https://community.openai.com/t/codex-work-gpt-6-astra-has-a-serious-model-downgrading-issue/1397173)）
- 工具触发："GPT dot 疑似造成封号，理由：累犯"（linux.do 帖）。【传闻】

**降级侧（2026-07～09 波次）**：

- 与"脏网络"的关系：V2EX 独立测试者在**完全干净的环境**（日区 Pro 20x、日本银行卡、日本手机号、KDDI 家宽、IP 与账单地址一致）仍被降级到 luna，且静置 10 分钟可恢复；另有用户报告**访问网页版 ChatGPT 后下一请求立刻变 luna**（单源观察）。说明脏网络可能是降级评分的因子之一，但干净网络不豁免。【实锤（干净环境仍降级）】（[V2EX 1241734](https://www.v2ex.com/t/1241734)）
- Pro 20x 高频重度用户是重灾区（五账号中两个持续降级的对照实验：[reddit 1wmk8i7](https://www.reddit.com/r/codex/comments/1wmk8i7/five_pro_20x_accounts_two_persistently_degraded/)）；但低用量干净账号也有约 1/5 请求被路由到 luna 的案例。【实锤】（V2EX 1241734 楼层 easy88866）
- 共享/拼车号池商家：13 家抽检 2 家指纹为 Luna。【实锤】（linux.do 2877508）

### 2.3 OpenAI 的判定信号（泄露/逆向所得）

- **官方明示的信号**：Codex CLI 会在命中时显示"⚠ Your account was flagged for potentially high-risk cyber activity and this request was routed to gpt-5.2 as a fallback."——**官方客户端存在"安全标记→回退路由"链路**（bug + safety-check 标签）。【实锤】（[openai/codex#12079](https://github.com/openai/codex/issues/12079)）
- 官方客服话术（TEB 贴出）："系统可能智能地在可用模型变体之间路由请求，以维持性能、可靠性或账号特定考量"——首次半官方确认"账号特定考量"参与路由。【实锤】（[community 1391818 #16](https://community.openai.com/t/possible-gpt-5-6-sol-routing-mismatch-selected-gpt-5-6-but-server-metadata-shows-gpt-5-5-mini/1391818)）
- turn-state 312 信号：cockpit-tools 曾按官方上游 `x-codex-turn-state` 长度判定（312=疑似风控，292/332=正常），**一天后失效**——OpenAI 监测社区检测手段并快速调整。【实锤】（[cockpit-tools releases](https://github.com/jlcodes99/cockpit-tools/releases)，v1.3.58→v1.3.59）
- Guardian v2：Codex 客户端遥测含 `codex.guardian_v2.connection.duration_ms`、`guardian_credits_requested`，存在名为 Guardian 的 v2 审查服务；抓包可见大量 Cloudflare 挑战，判定为"分数触阈后的临时降档，可恢复"。【实锤】（V2EX 1241734 静态分析；Guardian 具体功能未公开）
- 网页端路由与浏览器身份/会话/OBI/设备分配/实验 cohort 相关（见 §3.4.1 证据）。【实锤】（community 1391809）

### 2.4 封禁 vs 降级的表现差异

| 维度 | 封禁（ban） | 静默降级（shadow downgrade） |
| --- | --- | --- |
| 通知 | 停用邮件（"违反条款/网络滥用"），登录即被拦 | 无任何通知，订阅继续扣费 |
| UI 表现 | 无法登录/使用 | 模型选择器照常显示 Sol/Astra |
| 元数据 | — | `server_ste_metadata.model_slug` / `resolved_model_slug` 暴露真实模型（5.5-mini 等） |
| 体感 | 直接不可用 | 回复极快、拒绝思考、质量断崖（"降到 4o 级别"） |
| 伴随症状 | — | "Selected model is at capacity"（Pro 20x 高发）、"正在进一步审慎考虑此请求"横幅 |
| 恢复路径 | 申诉（误封可恢复）；明确违规几乎不可恢复 | 静置/换新浏览器环境/OpenAI 修复/未知原因自行恢复（有菲区 20x 数天后恢复案例：[linux.do 2877565](https://linux.do/t/topic/2877565)） |

OpenAI 支持对降级的标准回应：Pelican 测试非官方基准、不能证明路由；容量错误与审查提示不能证明账号被标记——**官方从不确认降级，只确认容量问题**（V2EX 1241734 Supplement 4；reddit 1wlk3ym《为什么 OpenAI 不确认》）。【实锤】

---

## 3. "降级到隐藏模型"的机制线索

### 3.1 历史案例（GPT-4 时代）

- **Lazy mode（2023-12）**：GPT-4 大规模"变懒"、拒写长代码。官方承认模型自 11 月 11 日未更新、"变懒非故意"；2024-02 Altman 发帖"应该没那么懒了"——**模型行为在权重不变的情况下被服务端配置改变**的最早公开案例。【实锤】（[PCMag](https://www.pcmag.com/news/openai-acknowledges-gpt-4-is-getting-lazy)、[Business Insider](https://www.businessinsider.com/chatgpt-openai-sam-altman-chatbot-less-lazy-after-users-complain-2024-2)）
- **行为漂移的学术证据**：MIT HDSR《How Is ChatGPT's Behavior Changing Over Time?》量化了 2023-03→06 GPT-4 行为显著漂移。【实锤】（[论文](https://hdsr.mitpress.mit.edu/pub/y95zitmz)）
- **GPT-4→GPT-3.5 降级传闻**：community 用户 abyss.of.abomination 称"模型降级是 GPT-4 时代的旧症状，当时人们发现选 GPT-4 实际给了 GPT-3.5 甚至更差"。【传闻】（[community 1397173 #12](https://community.openai.com/t/codex-work-gpt-6-astra-has-a-serious-model-downgrading-issue/1397173)）
- Santa mode 为 2023-12 的节日彩蛋（GPT-4 圣诞人格），**不是**惩罚路由；但用户任务中"类似 santa mode 的 A/B/惩罚路由"确有对应物：下述 2026 年的静默降级与官方称的"实验"。【实锤（彩蛋性质）/推测（类比）】

### 3.2 2026 年事件链（从 5.5-mini 到 Luna）

**阶段一：Pro→5.5-mini（7 月起）**。Pro 5x/20x 用户 HAR 抓包铁证：请求 `model_slug: gpt-5-6-pro`，响应 `resolved_model_slug: gpt-5-5-mini`，`plan_type: prolite`；UI 无任何降级提示；盲测排名 Pro 段位垫底（低于 Instant）；换账号/换卡/换 IP/升级 20x 均无效；支持确认"界面选择与服务器元数据不一致"并升级至路由/权限/计费团队。【实锤】（[community 1387941](https://community.openai.com/t/5-6-pro-model-has-been-automatically-downgraded-and-routed-to-the-5-5-mini-model-since-its-release/1387941)，含多个独立案例与 case 编号）

**阶段二：Sol→5.5-mini（8-20～25）**。Plus 账号批量复现：出站请求 `/backend-api/f/conversation` 中 `model: gpt-5-6-thinking`，而 `/ces/v1/telemetry/intake` 遥测 `turn_analytics.server_ste_metadata.model_slug: gpt-5-5-mini`；多集群（germanynorth、westus3）；有人 100% 每条必中、有人仅 Medium/High 档中招（Fast 正常）；**全新独立 Chromium 用户目录可恢复**（提示浏览器身份/会话/OBI/设备分配参与路由）；官方最终回帖"团队已在修复并部署"。【实锤】（[1391809](https://community.openai.com/t/chatgpt-web-requests-gpt-5-6-sol-but-server-resolves-gpt-5-5-mini-on-plus-accounts/1391809)、[1391818](https://community.openai.com/t/possible-gpt-5-6-sol-routing-mismatch-selected-gpt-5-6-but-server-metadata-shows-gpt-5-5-mini/1391818)；后续复发帖 [1394865](https://community.openai.com/t/regression-gpt-5-6-sol-thinking-extended-is-silently-routed-to-gpt-5-5-mini-again-confirmed-via-har-metadata/1394865)）

**阶段三：Astra/Sol→Luna（9-03 GPT-6 发布后）**。V2EX pwinner 的独立测试（干净日区 20x）：
- ModelTrace 指纹确认 astra/sol/terra/5.5 全部路由 luna；鹈鹕测试质量崩塌；**用量统计 astra 与 luna 调用次数几乎 1:1**（正常时 astra 自己会按需调 terra/luna，分布可预期）；
- **MITM 抓包实锤**："astra 直接路由 luna 是实锤的"（附抓包截图，`effort: medium, mode: standard`）；
- 风控时间窗：内部存在 10min/30min/1h/4h 惩罚性风控；**每 X 小时后第一次请求正常，随后降级**；关闭全部客户端静置 10 分钟可恢复，**一打开网页版 ChatGPT 立刻掉回 luna**（单源观察）；"遇到'正在进一步审慎考虑此请求'横幅=真 astra，直接秒答=大概率为 luna"；
- 静态分析 Codex 客户端发现 Guardian v2 遥测字段；大量 Cloudflare 挑战——判定为"分数触阈后的临时降档"；
- 猜测存在随时间衰减的风控：静置 2 天→可用 1 天→再降级。【实锤（指纹+MITM+统计）/【推测】（衰减模型）】（[V2EX 1241734](https://www.v2ex.com/t/1241734)）

**阶段三补充（复核新增，HAR 级独立实证）**：[community 1399076](https://community.openai.com/t/requested-gpt-6-astra-but-possibly-routed-to-gpt-5-6-luna-paying-users-deserve-transparency-and-accountability/1399076)（9-19~23）：
- sheng #6（9-20）：官方端点直连（chatgpt.com/backend-api/codex/responses，codex_cli_rs/0.155.0，无代理无中间件），请求 `gpt-6-astra`，**响应体 response.created/in_progress/completed 三处 model ID 均为 gpt-5.6-luna**；响应头 `x-codex-turn-state: 312 chars/11 blocks`、safety-buffering 头在位；**额度仅用 4%**（排除 Luna Reserve 兑底）；krasnik 评定「materially closes the provenance gap"。【实锤】
- L.stradud #12（9-21）：Pro $200/月，CLI 请求 astra xhigh，response.created.model=gpt-5.6-luna；**OpenAI 支持向其确认 response.created 的 model 字段即实际服务模型**、证据与「服务端路由/回退」一致。【实锤（半官方承认）】
- disunize #3：转 SparxieCX 报告——Codex 上 ~15% 的 astra 请求返回 Luna model ID（Pro 20x），按 astra 费率扣额度。【传闻】
- krasnik #18（9-23）重要限定：干净抓包中请求 astra→response.created.model=gpt-6-astra，**但同一响应也带 safety-buffering-faster-model: gpt-5.6-luna 头**——该头单独出现不等于本次响应被降级。【实锤】

**阶段四：显式化（9 月下旬）**。Reddit 报告 OpenAI 将静默降级部分替换为显式阻断（"Model capacity"报错不再偷偷换模型）。【传闻】（[reddit 1wnj1xe](https://www.reddit.com/r/codex/comments/1wnj1xe/openai_replaced_silent_model_degradation_with/)）

**并行的"合法降级"：Luna Reserve**。Tibo Sottiaux（Codex/ChatGPT 工程负责人）2026-08-27 在 X 确认：主力额度打空后 Codex 自动切到"Luna Reserve"备用档（记住原模型、临时替换、独立余量条），是降级兜底而非无限续杯；Luna 档不继承 Sol/Terra 的浏览器/网站检查能力——**Luna 作为官方兜底档的存在是实锤**，这使"被路由到 Luna"既可能是惩罚也可能是正常兜底，需结合额度状态判别。【实锤】（[Net.Coffee](https://ip.net.coffee/gpt/news/20260828a.html)）

**同模型"减配"：juice value（推理预算）**。社区通过提示词注入（"类似 SQL 注入的漏洞"，XML 代码提示在 Codex 有效、直接询问在网页端部分有效）读取模型内部运行时参数"juice 值"（0–512/960 分值越大越聪明）：
- 2026-03（GPT-5.4 时代）国内中转无法读出、官方网页 16 分、Codex 深度思考 512 满分（[CSDN](https://blog.csdn.net/m0_56232078) 引述，原文需自行核验）；【传闻】
- 2026-07：满血 5.5 xhigh=768，被灰度到 5.6-sol 的用户=128，缩水 6 倍（[CSDN IT界那些事儿](https://blog.csdn.net/) 转述）；【传闻】
- 2026-07-14：5.6 Sol Max 档 juice 960→128（-87%），同期上下文 372k→272k（-27%）；Tibo 回应"high/xhigh 档多智能体调用比预期多、auto-review 有浪费"——官方口径为成本/实验调整。【传闻→部分实锤】（[CSDN 162893239](https://blog.csdn.net/ZhengHuiNing/article/details/162893239)；多家媒体佐证；Tibo 回应与 372k→272k 变化有多个独立来源：[Threads](https://www.threads.com/@iamnanyi/post/Da_vATymdpW)）
- 社区流传的 juice 分档表：Luna（Low5/Medium4/High16/XHigh36/Max512）vs Sol（Low4/Medium12/High20/XHigh32/Max64）——注意两模型分值体系不可直接比较。【传闻】（[telegram 镜像](https://t.me/s/linuxdo_tg?after=368645)）

**turn-state 292/312/332 机制**（社区逆向，多个独立工具交叉）：
- Codex Responses 响应带 `X-Codex-Turn-State` 头（加密票据）。ccodex-sleep-state 归纳：个人账号 10 块/通常 292 字符为合格，11 块/312 字符不合格；Team/Business 12 块/332 字符合格，13 块/356 不合格。【实锤（多工具一致）】（[gylive/ccodex-sleep-state](https://github.com/gylive/ccodex-sleep-state)、[tzf1003/csss](https://github.com/tzf1003/csss)）
- cockpit-tools v1.3.58（9-22）按 312=疑似风控判定，v1.3.59（9-23）以"已失效"移除——信号存续不足一天。【实锤】（[releases](https://github.com/jlcodes99/cockpit-tools/releases)）
- Reddit r/codex 高热帖《The End of the Codex Era》："一旦那个 header 长度从 292 变成 312，你就会被降级到 Luna Low，且仍按所选模型计费"。【传闻】（[reddit 1wkwdfl](https://www.reddit.com/r/codex/comments/1wkwdfl/the_end_of_the_codex_era_ive_completely_lost/)）
- X 长文《万字拆解》提出"HTTP 292 状态码 + current_turn_state 通行证"模型：state TTL≈1 小时、签发与 IP 质量/账号等级/全局水位相关、state 不与 IP 锁死（可用干净住宅 IP 采集后注入日常线路）；并给出 codex-state-kit（gpt-load+keeper.py+inject_proxy+Clash）自动化方案。**注意**：该文的"292 状态码"表述与 GitHub 工具一致的"turn-state 长度 292 字符"存在出入，疑为演绎/混淆，其 IP 分级表（美国住宅/原生 IPv6 极高、机房/共享代理极低）与其它来源一致但属单源。【传闻】（[X @ElowenY20119](https://x.com/i/article/2100808799389208987)）
- csss 工具用隐藏探针"最新的 iPhone 型号是什么"做质量门（iPhone 17=通过、16=降智、15=更严重）——**知识截止探针**：降级模型知识截止更旧。state 注入按"账号+模型"隔离，TTL 约 5 分钟保守续期。【实锤】（[tzf1003/csss](https://github.com/tzf1003/csss)）

### 3.3 GPT-5.6 家族命名与 Luna 的社区定位

| 模型 | 官方定位 | 来源 |
| --- | --- | --- |
| Sol | frontier（最难任务） | [OpenAI GPT-5.6 发布页](https://openai.com/index/gpt-5-6/) |
| Terra | value（生产/性价比） | 同上；[llmrumors](https://www.llmrumors.com/news/openai-gpt56-sol-terra-luna-government-preview) |
| Luna | volume/nano 档（高频、低延迟、成本敏感），"大致对应早期 GPT-5 家族的 nano 档" | [官方模型文档](https://developers.openai.com/api/docs/models/gpt-5.6-luna)、[OpenRouter](https://openrouter.ai/openai/gpt-5.6-luna) |
| Astra | GPT-6 旗舰（Pro 档由其驱动） | [OpenAI 帮助中心](https://community.openai.com/t/codex-work-gpt-6-astra-has-a-serious-model-downgrading-issue/1397173/17 引述) |

- 免费版默认模型 2026-08-06 起切到 Luna（[OpenAI 博客](https://openai.com/index/improving-gpt-5-6-sol-in-chatgpt/)）；OpenAI 曾降价 Luna 80%/Terra 20%（[X @OpenAI](https://x.com/OpenAI/status/2082878156483219672)）。【实锤】
- **Luna 是否"影子/惩罚模型"**：Luna 本身是公开的低档模型，但社区普遍把"被路由到 Luna"视为惩罚信号；区分要点——①额度打空后的 Luna Reserve 是官方明示兜底；②**Codex 的 auto-review（"帮我批准"）后台本来就合法使用 5.6-luna**，所以用量统计里出现 luna ≠ 一定被降级（V2EX 楼层 Alexliu/yihy8023 指出，pwinner 以 MITM 反驳"画鹈鹕不可能用子代理/auto-review"）；③指纹命中 luna + 无 auto-review + 高置信度才是降级实锤。【实锤 + 推测】
- Reddit 讨论："GPT-6 Luna Pro is a Mode, Not a Model"（[orcarouter](https://www.orcarouter.ai/blog/gpt-6-luna-pro-explained)）——Luna 与推理档位（Luna Low/Medium/Max 等）组合出多档体验。【传闻】

### 3.4 "选 A 模型实际返回 B 模型"的检测方法汇总（按证据强度排序）

1. **服务端元数据（最硬）**：网页 HAR 中 `/backend-api/f/conversation` 请求 `model` 字段 vs `/ces/v1/telemetry/intake` 的 `server_ste_metadata.model_slug`；对话元数据 `resolved_model_slug`；Codex 侧经 app-server 的 model-reroute 事件（codexometer 可实时捕获"实际服务模型"并按其计价：[codexometer](https://github.com/merefield/codexometer)）。【实锤】
2. **行为指纹**：lm.ikale.io / lmfpd / ModelTrace 随机数指纹（§1）。【实锤】
3. **知识截止探针**：问"最新 iPhone 型号"（17=正常/16=降智/15=更糟）；"你的知识截止"（答 June 2024 → 降级）。【实锤（csss 用作质量门）】
4. **能力题**：鹈鹕骑自行车 SVG（社区标准测试，OpenAI 客服明确不认可其证据效力）、糖果计数题、蒙娜丽莎 canvas。【实锤（广泛使用）/官方不认】
5. **模型自述 prompt**：AI超元域 流传的"禁止联网搜索……请仅凭模型参数中已有内部知识回答当前你的模型"——模型可能误报自述，仅作初筛。【传闻】（[X @AISuperDomain](https://x.com/AISuperDomain/status/2085279137736839212)）
6. **turn-state 长度**：312=疑似风控（已失效，OpenAI 快速调整过）。【实锤（历史有效）】
7. **用量统计交叉**：codex analyze/用量页 luna 调用占比异常（排除 auto-review 后）。【实锤】
8. **juice 值**：推理预算注入读取（可能已被修补）。【传闻】
9. **体感信号**：秒回、拒绝思考、"审慎考虑"横幅后秒答、at capacity。【传闻】

### 3.5 恢复案例

- 全新独立浏览器环境（独立 Chromium user-data 目录、无同步/扩展）→ 同账号恢复正常路由（Sol 事件中多人复现，官方修复帖确认）。【实锤】（[1391809 #4](https://community.openai.com/t/chatgpt-web-requests-gpt-5-6-sol-but-server-resolves-gpt-5-5-mini-on-plus-accounts/1391809)）
- 静置：关全部客户端静置（10min 起）恢复，但访问网页版立刻再触发。【实锤】（V2EX 1241734）
- OpenAI 服务端修复：Sol→5.5-mini 官方"已部署修复"；linux.do《【openai已修复】确认codex 5.6-sol降智路由到luna的方式》发帖人称修复后 team 号测试无降智。【实锤】（官方修复转达出自 [1391809 #14](https://community.openai.com/t/chatgpt-web-requests-gpt-5-6-sol-but-server-resolves-gpt-5-5-mini-on-plus-accounts/1391809)、[linux.do 2850418](https://linux.do/t/topic/2850418)）
- 自行恢复：菲区 20x 降智数天后未知原因恢复（[linux.do 2877565](https://linux.do/t/topic/2877565)）。【传闻】
- 申诉：封号误判可申诉恢复（1-15 工作日）；降级类申诉官方一律不承认降级存在。【实锤】（ofox + V2EX 1241734）
- 对抗性恢复（社区工具）：ccodex-sleep-state / csss / codex-state-kit 通过干净出口采集合格 state 后注入 `x-codex-turn-state` 绕过降级队列；keeper 心跳续期。作者自述"不保证取得指定 state 或提升模型质量"。【实锤（工具存在且开源）/有效性传闻】

---

## 4. OpenAI 反滥用技术公开资料

### 4.1 chatgpt.com 请求防线（逆向资料）

对 chatgpt.com 前端的逆向（ChatGPTReversed，教育项目）披露的调用链：【实锤】（[gin337/ChatGPTReversed](https://github.com/gin337/ChatGPTReversed)）

- 会话前先 POST `https://chatgpt.com/backend-api/sentinel/chat-requirements`，传入 Requirements Token（token x），返回 Required Requirements Token（token y）与配置：
  ```json
  {"persona":"chatgpt-freeaccount","token":"y","arkose":{},"turnstile":{},
   "proofofwork":{"required":true,"seed":"0.8118...","difficulty":"073682"}}
  ```
- **Proof of Work**：前端 `_generateAnswer` 用 hash-wasm 以 seed+difficulty 求解哈希，且**混入设备参数（屏幕尺寸、时区、CPU 核心数等）参与计算**——PoW 本身携带设备指纹成分。
- 会话请求需同时携带：Authorization（JWT）、csrf-token、session-token、Requirements-Token、Proof Token。
- `persona` 字段标识账号类别（如 chatgpt-freeaccount）；Arkose 与 Turnstile 槽位按需启用。
- 该项目 2025-12-30 更新注记："OpenAI 最近几周改了前端流程"——防线持续演进。

### 4.2 设备指纹

- **Arkose Device ID**（Arkose Labs，2024-11 发布）：硬件属性+浏览器配置+会话遥测构建动态指纹，精确匹配 + AI 相似度检测双层，跨会话/账号追踪威胁行为者；官方称 <50ms、抗指纹漂移。ChatGPT 是 Arkose 的公开客户（注册/登录验证）。【实锤】（[Arkose 官网](https://www.arkoselabs.com/arkose-device-id)、[发布稿](https://www.businesswire.com/news/home/20241119554933/en/Arkose-Labs-Launches-Arkose-Device-ID-A-Dual-Method-Approach-to-Precise-Persistent-Device-Identification)、[Help Net Security](https://www.helpnetsecurity.com/2026/03/04/arkose-labs-device-id/)）
- **OBI / 设备分配**：Sol→5.5-mini 事件中，受影响会话伴随 "OBI synchronization is not available"、"persona: chatgpt-noauth" 等身份同步错误，且换全新浏览器环境即恢复——网页端路由与浏览器身份/会话状态/设备 ID/OBI/实验 cohort 相关联。【实锤】（[1391809](https://community.openai.com/t/chatgpt-web-requests-gpt-5-6-sol-but-server-resolves-gpt-5-5-mini-on-plus-accounts/1391809)）
- 一机多号（同一浏览器指纹登录两个以上账号）被列为封号信号。【实锤】（ofox）

### 4.3 IP 信誉 / 数据中心 IP 检测（对 VPS 出口的影响）

- **Cloudflare 1020**：chatgpt.com 位于 Cloudflare 后，VPN/数据中心 IP 常见 "Access denied error code 1020"（ASN 级封锁）。【实锤】（[Quora 案例汇总](https://www.quora.com/I-always-use-a-VPN-but-I-cant-go-to-chat-GPT-from-the-VPN-access-denied-error-code-1020-What-can-be-done-about-this)）
- **支付/订阅环节**："OpenAI 和 Stripe 早就学会识别数据中心 IP"，触发 "This service is not available..."。【实锤】（[proxycove](https://proxycove.com/en/blog/chatgpt-plus-proxy-subscription-blocked-country)）
- **IP 信誉打分维度**（社区归纳，与 ofox/X 文章交叉）：共享代理出口连坐、短时跨国跳变、公有云 ASN 段标记为非住宅、接码平台历史 IP 段；对 VPS 出口的影响：机房/共享出口是风控评分的不利因子，住宅/原生 IPv6 为有利因子——但 IP 质量既非降级的充分也非必要条件（完美住宅环境仍降级、机房 IP 也有正常服务的反例），工具作者亦明确免责。【方向性因子（多源一致）；「基本必进」表述已被反证推翻】（[ofox](https://blog.csdn.net/ofox_ai_hunter/article/details/161823409)、[X 万字拆解](https://x.com/i/article/2100808799389208987)、[Net.Coffee（出口 IP 与风控关系）](https://ip.net.coffee/gpt/news/20260828a.html)）
- Codex 桌面端"起不了新任务"未必是额度，可能是出口 IP/分流触发风控——社区已有专门的 GPT/Codex IP 检测工具（ip.net.coffee）区分"限流挡下"vs"风控挡下"。【实锤（工具存在）】

### 4.4 OAuth token 共享检测的一般机制

OpenAI 未公开其内部机制；业界通行做法（可据以推断 OpenAI 的检测面）：【实锤（业界机制）/推测（OpenAI 具体实现）】

- **Refresh token 轮换与重用检测**：一次性 refresh token 被第二次使用 → 判定 token 被盗/共享，吊销整个家族（Auth0 Detection Catalog 有现成检测器：并发使用、异常地理位置、token replay）。来源：[Auth0](https://auth0.com/blog/refresh-token-security-detecting-hijacking-and-misuse-with-auth0/)、[Obsidian Security](https://www.obsidiansecurity.com/blog/token-replay-attacks-detection-prevention)。
- **多 IP 并发同一 refresh token**：同一凭据短时间内从多个 ASN/地理并发刷新，是教科书级异常；学术界已有 JWT+refresh token 监控系统的设计（[论文](https://journals.riverpublishers.com/index.php/JCSANDM/article/view/27697/22597)）。
- 映射到 Codex 场景：Codex CLI 用 ChatGPT OAuth 登录，同一 auth.json 被多机/多 IP 同时使用（拼车、sub2api、号池网关）会天然产生"同 token 多 IP 并发 + 无浏览器指纹 + 高 QPS"三重信号——与 §2.2 封号特征完全吻合。【推测】
- 佐证：OpenAI 2026-05 给 Codex 登录加强制手机验证（一号一绑），直接打击号池共享；manifest.build 明确"订阅严格个人使用，账号池/轮换、CI 自动化、服务他人、商业转售均属违规"。【实锤】（ofox、[manifest.build](https://manifest.build/blog/banned-from-chatgpt-subscriptions/)）

---

## 5. 综合结论

1. **"GPT-5.6-LUNA 是降级路由目标"成立**，但需精确定义：Luna 是公开的低档模型；被社区视为"惩罚路由"的是**在用户选择 Sol/Astra/Pro 且额度未耗尽时，服务端静默将请求解析为 Luna（或 5.5-mini/4o）并按原模型计费**。证据链：HAR 元数据（resolved_model_slug）+ 行为指纹（ModelTrace/lm-detector 命中 luna）+ MITM 抓包 + 用量统计异常，四类独立证据在 2026-07～09 多个账号、多个平台复现；OpenAI 官方对 5.6-sol→5.5-mini 一轮承认并修复，对其余轮次一律以"容量/实验"话术回应。【实锤】
2. **降级不是单一机制**，至少有五种叠加：容量回退（官方兜底/Luna Reserve）、账号风控降档（Guardian/turn-state 分数，可恢复、时间衰减）、灰度实验（juice 值压缩，官方称"实验"）、bug（5.6-sol→5.5-mini 已修复）、以及中转站主动掺假（grok 冒充 codex 等，linux.do 有曝光帖）。
3. **lm.ikale.io 是目前最系统的行为指纹工具**：53 模型参考库含 GPT-5.6 全家官方直连指纹，CLI 支持直连 Codex 订阅采样，可作为 codex-proxy-rs 这类网关的"模型保真"自检工具；但其结果为封闭集合排序，需与服务端元数据、用量统计交叉使用。
4. **OpenAI 的对抗迭代速度很快**：312 信号被社区工具采用一天后即失效；网页 PoW/前端流程 2025-12 又改版；juice 注入点被陆续修补。任何单一检测/对抗手段都有短保质期，工程上应做多信号冗余。
5. **风控信号面**：IP 信誉（住宅/机房分级）、设备指纹（Arkose Device ID、OBI、PoW 内嵌设备参数）、支付关联（同卡多号）、行为模式（间隔规整、并发、多机）、OAuth 并发（同 token 多 IP）、重置卡滥用、以及"网页版访问"这类会话级触发器。共享/拼车/反代订阅在这些维度上几乎必然暴露多个信号。

## 附：本次实测记录（lm.ikale.io / lmfpd）

| 步骤 | 命令/操作 | 结果 |
| --- | --- | --- |
| 打开站点 | agent-browser open https://lm.ikale.io/#/?mode=api | 正常加载，含"手动/API"双模式、CLI 卡片、API 配置（OpenRouter 预置）、3 道多语言整数挑战 |
| 样本库 | 打开 #/library | 53 模型 1948 样本，含 gpt-5.6-luna/sol/terra 等 |
| 参考库数据 | curl raw.githubusercontent.com/.../data/unified_bank.json | 11.8MB，GPT-5.6 三模型各 36 样本、来源 openai/direct |
| API 代理 | curl -X POST https://lm.ikale.io/api/proxy（伪造 key 转发到 api.openai.com） | HTTP 401 "Supply an API key for the selected service."，证明转发链路工作 |
| CLI | npx -y lmfpd@latest --help | 正常输出完整帮助（Responses/CC/Messages、--subscription、sample/enroll/retrain 等） |

未执行真实模型检测（无可用 API Key/Codex 登录凭据）；复现步骤见 §1.7。
