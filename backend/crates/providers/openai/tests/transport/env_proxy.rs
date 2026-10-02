//! 上游出口环境代理策略（`CODEX_UPSTREAM_ENV_PROXY`）的选路矩阵测试。
//!
//! 环境变量只在子进程内生效：父进程先起好各出口/直连监听器，再用 CASE_ENV
//! 驱动同一测试二进制隔离运行单个场景，避免共享进程的代理环境变量与进程级
//! 缓存（策略 OnceLock、reqwest 客户端缓存）互相污染。

use super::*;
use gateway_core::{
    account::{CredentialRevision, OutboundProxy, ProviderAccount, ProviderAccountId},
    routing::ProviderKind,
};

const CASE_ENV: &str = "CODEX_PROXY_TEST_UPSTREAM_ENV_PROXY_CASE";
const DIRECT_URL_ENV: &str = "CODEX_PROXY_TEST_UPSTREAM_ENV_DIRECT_URL";
const ACCOUNT_URL_ENV: &str = "CODEX_PROXY_TEST_UPSTREAM_ENV_ACCOUNT_URL";
const CASE_COMPLETED: &str = "upstream-env-proxy-case-completed:";
/// 子进程必须清理的代理环境变量，保证场景只受本矩阵注入的值影响。
const PROXY_ENV_KEYS: [&str; 8] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];
/// 单场景端到端预算：正确路径毫秒级完成，该值只需兜住错误路径的快速失败。
const CASE_DEADLINE: Duration = Duration::from_secs(20);

fn account(id: &str, proxy: Option<&str>) -> ProviderAccount {
    ProviderAccount::new(
        ProviderAccountId::new(format!("acct_{id}")).unwrap(),
        ProviderKind::new("openai").unwrap(),
        id.to_owned(),
        Some(id.to_owned()),
        "oauth".to_owned(),
        CredentialRevision::new(1).unwrap(),
        None,
    )
    .with_outbound_proxy(proxy.map(|value| OutboundProxy::parse(value).unwrap()))
}

/// 直连 HTTP 出口：接收 origin-form 请求并返回带标签的 SSE 应答。
async fn http_exit(label: &'static str) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_http_request_with_body(&mut stream).await;
        let body = format!(
            "event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{label}\",\"status\":\"completed\",\"output\":[],\"usage\":{{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}}}}\n\n"
        );
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let header_end = request
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap();
        String::from_utf8(request[..header_end].to_vec()).unwrap()
    });
    (format!("http://{address}"), task)
}

/// 经 HTTP CONNECT 隧道的 WebSocket 出口（环境代理与账号代理共用形态）。
async fn ws_proxy_exit(label: &'static str) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let connect = read_http_request(&mut stream).await;
        stream
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let mut websocket = accept_codex_test_websocket(stream).await;
        assert!(matches!(websocket.next().await, Some(Ok(Message::Text(_)))));
        websocket
            .send(Message::Text(
                completed_websocket_response(label, 2, 1).into(),
            ))
            .await
            .unwrap();
        connect
    });
    (format!("http://{address}"), task)
}

/// 直连 WebSocket 出口：不经任何代理，直接完成 opening 与响应回放。
async fn ws_direct_exit(label: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut websocket = accept_codex_test_websocket(stream).await;
        assert!(matches!(websocket.next().await, Some(Ok(Message::Text(_)))));
        websocket
            .send(Message::Text(
                completed_websocket_response(label, 2, 1).into(),
            ))
            .await
            .unwrap();
    });
    (format!("http://{address}"), task)
}

/// 必须保持静默的出口：任何连接立即断开并计数，用于「不应被使用」的否定断言。
async fn silent_exit() -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&count);
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            // 立即断开让错误路由快速失败，而不是等连接超时。
            drop(socket);
        }
    });
    (url, count, task)
}

/// 子进程断言已结束后确认静默出口计数为 0，并终止监听任务。
async fn assert_silent(name: &str, count: &AtomicUsize, task: &tokio::task::JoinHandle<()>) {
    // 子进程退出后其套接字已关闭；短暂窗口让尚在队列中的 accept 完成计数。
    tokio::time::sleep(Duration::from_millis(50)).await;
    let observed = count.load(Ordering::SeqCst);
    assert_eq!(observed, 0, "{name} must stay silent, saw {observed}");
    task.abort();
}

/// 子进程内跑生产 HTTP 出口路径，返回响应体。
async fn run_http_child(base_url: &str, account_proxy_url: Option<&str>) -> String {
    let base = CodexBackendClient::new(
        provider_openai::transport::build_reqwest_client().unwrap(),
        base_url,
        test_wire_profile(),
    );
    let client = base
        .for_account(&account("env-matrix", account_proxy_url))
        .unwrap();
    let mut request = codex_request("gpt-5.5", "", Vec::new());
    request.force_http_sse = true;
    let response = timeout(
        CASE_DEADLINE,
        client.create_response(
            &request,
            request_context("req_env_http", Some("env-matrix")),
        ),
    )
    .await
    .expect("HTTP case completes in time")
    .expect("HTTP case succeeds");
    response.body
}

/// 子进程内跑生产 WebSocket 出口路径（池化连接），返回响应体。
async fn run_ws_child(base_url: &str, account_proxy_url: Option<&str>) -> String {
    let pool = Arc::new(CodexWebSocketPool::new(Duration::from_mins(1)));
    let base = CodexBackendClient::new(
        provider_openai::transport::build_reqwest_client().unwrap(),
        base_url,
        test_wire_profile(),
    )
    .with_websocket_pool(pool);
    let client = base
        .for_account(&account("env-matrix", account_proxy_url))
        .unwrap();
    let mut request = codex_request("gpt-5.5", "", Vec::new());
    request.set_previous_response_id(Some("resp_previous".to_owned()));
    request.previous_response_scope = Some(PreviousResponseScope::Persisted);
    let response = timeout(
        CASE_DEADLINE,
        client.create_response(&request, request_context("req_env_ws", Some("env-matrix"))),
    )
    .await
    .expect("WebSocket case completes in time")
    .expect("WebSocket case succeeds");
    response.body
}

/// 以干净代理环境运行单个子场景，并断言子进程恰好完成一次。
async fn run_case(case: &str, extra_env: Vec<(&str, String)>) {
    let mut command = Command::new(std::env::current_exe().expect("current test binary path"));
    command
        .args([
            "--exact",
            "transport::env_proxy::upstream_env_proxy_selection_matrix",
            "--nocapture",
        ])
        .env(CASE_ENV, case);
    for key in PROXY_ENV_KEYS {
        command.env_remove(key);
    }
    command.env_remove(provider_openai::transport::client::UPSTREAM_ENV_PROXY_ENV);
    for (key, value) in extra_env {
        command.env(key, value);
    }
    // spawn_blocking 避免阻塞 current_thread 运行时，父进程的监听器任务得以服务子进程。
    let output =
        tokio::task::spawn_blocking(move || command.output().expect("run isolated env proxy case"))
            .await
            .expect("join isolated env proxy case");
    assert!(
        output.status.success(),
        "isolated env proxy case {case} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // libtest 匹配零个测试也返回成功；完成标记证明该环境分支执行了生产断言。
    let stdout = String::from_utf8_lossy(&output.stdout);
    let completed = format!("{CASE_COMPLETED}{case}");
    assert_eq!(
        stdout.lines().filter(|line| *line == completed).count(),
        1,
        "isolated env proxy case {case} did not complete exactly once\nstdout:\n{stdout}"
    );
}

/// 服务端任务必须在场景预算内完成，错误路由的挂起以超时形式暴露。
async fn join_server<T>(name: &str, task: tokio::task::JoinHandle<T>) -> T {
    timeout(CASE_DEADLINE, task)
        .await
        .unwrap_or_else(|_| panic!("{name} server task did not finish in time"))
        .expect("{name} server task must not panic")
}

#[test]
fn upstream_env_proxy_flag_parses_only_explicit_enable_values() {
    use provider_openai::transport::client::upstream_env_proxy_flag_enabled;
    assert!(!upstream_env_proxy_flag_enabled(None));
    for off in [
        "", "0", "false", "FALSE", "no", "on", "yes", "2", "true-ish", " ",
    ] {
        assert!(
            !upstream_env_proxy_flag_enabled(Some(off)),
            "{off:?} must not enable env proxy inheritance"
        );
    }
    for on in ["1", "true", "TRUE", "True", " 1 ", "\ttrue\n"] {
        assert!(
            upstream_env_proxy_flag_enabled(Some(on)),
            "{on:?} must enable env proxy inheritance"
        );
    }
}

/// 选路矩阵：代理环境变量存在/不存在 × 账号代理/直连 × HTTP/WS × 开关。
/// 子进程断言「正确出口返回唯一标签」，父进程断言「错误出口保持静默」。
#[tokio::test]
async fn upstream_env_proxy_selection_matrix() {
    if let Ok(case) = std::env::var(CASE_ENV) {
        let direct_url = || std::env::var(DIRECT_URL_ENV).expect("direct URL for the case");
        let account_url = || std::env::var(ACCOUNT_URL_ENV).expect("account URL for the case");
        let (body, expected) = match case.as_str() {
            "http_default_ignores_env_proxy" => (
                run_http_child(&direct_url(), None).await,
                "resp_http_direct_default",
            ),
            "http_optin_uses_env_proxy" => (
                run_http_child(&direct_url(), None).await,
                "resp_http_env_optin",
            ),
            "http_account_proxy_wins_over_env" => (
                run_http_child(&direct_url(), Some(&account_url())).await,
                "resp_http_account_wins",
            ),
            "ws_default_ignores_env_proxy" => (
                run_ws_child(&direct_url(), None).await,
                "resp_ws_direct_default",
            ),
            "ws_optin_uses_env_proxy" => (
                run_ws_child("http://upstream.invalid", None).await,
                "resp_ws_env_optin",
            ),
            "ws_account_proxy_wins_over_env" => (
                run_ws_child("http://upstream.invalid", Some(&account_url())).await,
                "resp_ws_account_wins",
            ),
            _ => panic!("unknown upstream env proxy case: {case}"),
        };
        assert!(
            body.contains(expected),
            "case {case} must be served by the expected exit; body: {body}"
        );
        println!("\n{CASE_COMPLETED}{case}");
        return;
    }

    // 默认策略：代理环境变量存在也被忽略，HTTP 直连打 origin-form 请求。
    {
        let (direct_url, direct) = http_exit("resp_http_direct_default").await;
        let (env_url, env_count, env_task) = silent_exit().await;
        run_case(
            "http_default_ignores_env_proxy",
            vec![
                (DIRECT_URL_ENV, direct_url),
                ("HTTP_PROXY", env_url.clone()),
                ("http_proxy", env_url),
            ],
        )
        .await;
        let head = join_server("http direct exit", direct).await;
        assert!(
            head.starts_with("POST /codex/responses HTTP/1.1"),
            "default HTTP egress must dial the target directly: {head}"
        );
        assert_silent("env proxy under default policy", &env_count, &env_task).await;
    }

    // 显式开启：直连账号的 HTTP 请求经环境代理转发（绝对形式 URI）。
    {
        let (env_url, env) = http_exit("resp_http_env_optin").await;
        let (direct_url, direct_count, direct_task) = silent_exit().await;
        run_case(
            "http_optin_uses_env_proxy",
            vec![
                (DIRECT_URL_ENV, direct_url),
                ("HTTP_PROXY", env_url.clone()),
                ("http_proxy", env_url),
                (
                    provider_openai::transport::client::UPSTREAM_ENV_PROXY_ENV,
                    "1".to_owned(),
                ),
            ],
        )
        .await;
        let head = join_server("env proxy exit", env).await;
        assert!(
            head.starts_with("POST http://"),
            "opt-in HTTP egress must forward absolute-form request via env proxy: {head}"
        );
        assert_silent(
            "direct target under opt-in policy",
            &direct_count,
            &direct_task,
        )
        .await;
    }

    // 账号显式代理独占出口：开关开启也不与环境代理叠加。
    {
        let (account_url, account) = http_exit("resp_http_account_wins").await;
        let (env_url, env_count, env_task) = silent_exit().await;
        let (target_url, target_count, target_task) = silent_exit().await;
        run_case(
            "http_account_proxy_wins_over_env",
            vec![
                (DIRECT_URL_ENV, target_url),
                (ACCOUNT_URL_ENV, account_url),
                ("HTTP_PROXY", env_url.clone()),
                ("http_proxy", env_url),
                (
                    provider_openai::transport::client::UPSTREAM_ENV_PROXY_ENV,
                    "1".to_owned(),
                ),
            ],
        )
        .await;
        let head = join_server("account proxy exit", account).await;
        assert!(
            head.starts_with("POST http://"),
            "explicit account proxy must serve absolute-form request: {head}"
        );
        assert_silent("env proxy under account proxy", &env_count, &env_task).await;
        assert_silent(
            "direct target under account proxy",
            &target_count,
            &target_task,
        )
        .await;
    }

    // 默认策略：存在环境代理时 WS 直连账号不继承，改走显式直连拨号。
    {
        let (direct_url, direct) = ws_direct_exit("resp_ws_direct_default").await;
        let (env_url, env_count, env_task) = silent_exit().await;
        run_case(
            "ws_default_ignores_env_proxy",
            vec![
                (DIRECT_URL_ENV, direct_url),
                ("HTTP_PROXY", env_url.clone()),
                ("http_proxy", env_url),
            ],
        )
        .await;
        // 任务内完成 WS opening 断言；若错误路由出现 CONNECT 会直接失败。
        join_server("ws direct exit", direct).await;
        assert_silent("env proxy under default policy", &env_count, &env_task).await;
    }

    // 显式开启：WS 走 tungstenite 原生拨号，经环境代理 CONNECT 隧道（对齐官方）。
    {
        let (env_url, env) = ws_proxy_exit("resp_ws_env_optin").await;
        run_case(
            "ws_optin_uses_env_proxy",
            vec![
                ("HTTP_PROXY", env_url.clone()),
                ("http_proxy", env_url),
                (
                    provider_openai::transport::client::UPSTREAM_ENV_PROXY_ENV,
                    "1".to_owned(),
                ),
            ],
        )
        .await;
        let head = join_server("env proxy WS exit", env).await;
        assert!(
            head.starts_with("CONNECT upstream.invalid:80"),
            "opt-in WS egress must tunnel through env proxy: {head}"
        );
    }

    // 账号显式代理独占 WS 出口：开关开启时环境代理仍不得参与。
    {
        let (account_url, account) = ws_proxy_exit("resp_ws_account_wins").await;
        let (env_url, env_count, env_task) = silent_exit().await;
        run_case(
            "ws_account_proxy_wins_over_env",
            vec![
                (ACCOUNT_URL_ENV, account_url),
                ("HTTP_PROXY", env_url.clone()),
                ("http_proxy", env_url),
                (
                    provider_openai::transport::client::UPSTREAM_ENV_PROXY_ENV,
                    "1".to_owned(),
                ),
            ],
        )
        .await;
        let head = join_server("account proxy WS exit", account).await;
        assert!(
            head.starts_with("CONNECT upstream.invalid:80"),
            "explicit account proxy must carry the WS tunnel: {head}"
        );
        assert_silent("env proxy under account proxy", &env_count, &env_task).await;
    }
}
