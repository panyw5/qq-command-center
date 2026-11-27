use std::fs;
use std::path::PathBuf;

/// TUI 客户端连接配置。
///
/// 解析优先级：
/// - host：`--host` > `QCE_HOST` > `~/.config/qce-tui/config.json` > 默认 loopback
/// - token：`--token` > `QCE_TOKEN` > `~/.config/qce-tui/config.json` >
///   本机 `~/.qq-chat-exporter/security.json`（Docker 部署时读不到，用前两种）
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub token: String,
}

const DEFAULT_HOST: &str = "http://127.0.0.1:40653";

pub fn resolve(args: &[String]) -> Result<Config, String> {
    let mut arg_host = None;
    let mut arg_token = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--host" => arg_host = iter.next().cloned(),
            "--token" => arg_token = iter.next().cloned(),
            "-h" | "--help" => return Err(help()),
            other => return Err(format!("未知参数: {other}\n\n{}", help())),
        }
    }

    let file = read_config_file();
    let file_host = file
        .get("host")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let file_token = file
        .get("token")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    let host = arg_host
        .or_else(|| std::env::var("QCE_HOST").ok())
        .or(file_host)
        .unwrap_or_else(|| DEFAULT_HOST.to_string());
    let host = host.trim_end_matches('/').to_string();

    let token = arg_token
        .or_else(|| std::env::var("QCE_TOKEN").ok())
        .or(file_token)
        .or_else(read_local_security_token)
        .ok_or_else(|| {
            format!(
                "找不到访问令牌。\n\n\
                 任选其一提供：\n  \
                 --token <token>\n  \
                 QCE_TOKEN=<token> qce-tui\n  \
                 写入 {} ({{\"token\": \"...\"}})\n\n\
                 Docker 部署时令牌在容器内，可用：\n  \
                 docker exec napcat-qce cat /app/.qq-chat-exporter/security.json",
                config_path().display()
            )
        })?;

    Ok(Config { host, token })
}

/// 宽容版解析：`status`/`restart` 等诊断命令即使拿不到 token 也要继续跑。
pub fn resolve_permissive() -> (String, Option<String>) {
    match resolve(&[]) {
        Ok(c) => (c.host, Some(c.token)),
        Err(_) => {
            let file = read_config_file();
            let host = std::env::var("QCE_HOST")
                .ok()
                .or_else(|| {
                    file.get("host")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| DEFAULT_HOST.to_string());
            (host.trim_end_matches('/').to_string(), None)
        }
    }
}

fn help() -> String {
    "qce — QQ Chat Exporter 命令行入口\n\n\
     用法:\n  \
     qce tui [--host http://127.0.0.1:40653] [--token <token>]   打开聊天 TUI\n  \
     qce login [--webui <url>] [--webui-token <token>]          打开 NapCat WebUI 登录页\n  \
     qce export [--open]                                       打印导出界面 token 与网址\n\n\
     不带子命令等同于 qce tui。\n\
     环境变量: QCE_HOST, QCE_TOKEN, QCE_WEBUI_URL, QCE_WEBUI_TOKEN"
        .to_string()
}

/// `qce-tui login`：解析 `NapCat` `WebUI` 登录页 URL 并附 token 参数。
///
/// token 解析顺序：--webui-token > `QCE_WEBUI_TOKEN` > config.json
/// `webui_token` 字段 > 本机 webui.json 探测（Docker 挂载目录等）。
/// URL 解析顺序：--webui > `QCE_WEBUI_URL` > config.json `webui_url`
/// > 默认 <http://127.0.0.1:6099/webui>。
pub fn webui_login_url(args: &[String]) -> Result<String, String> {
    let mut arg_url = None;
    let mut arg_token = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--webui" => arg_url = iter.next().cloned(),
            "--webui-token" => arg_token = iter.next().cloned(),
            other => return Err(format!("未知参数: {other}")),
        }
    }

    let file = read_config_file();
    let url = arg_url
        .or_else(|| std::env::var("QCE_WEBUI_URL").ok())
        .or_else(|| {
            file.get("webui_url")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "http://127.0.0.1:6099/webui".to_string());
    let url = url.trim_end_matches('/').to_string();

    let token = arg_token
        .or_else(|| std::env::var("QCE_WEBUI_TOKEN").ok())
        .or_else(|| {
            file.get("webui_token")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .or_else(read_webui_token);

    Ok(match token {
        Some(t) => format!("{url}?token={t}"),
        // 没带 token 也能打开，WebUI 页面会要求输入（docker logs 里可查）。
        None => url,
    })
}

/// `napcat-qce` Docker 容器是否在运行。
///
/// Docker 与本机补丁版 QQ 同一时刻只能一方占用 6099/40653，谁在线谁的令牌才有效，
/// 因此用容器存活状态决定候选文件的优先顺序。
fn docker_napcat_running() -> bool {
    std::process::Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", "napcat-qce"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// 在本机常见位置探测 `NapCat` webui.json。
///
/// Docker 部署挂载目录与本机补丁版 QQ 的运行目录都会在磁盘上留文件，
/// 按容器在线状态排序后取第一份有效 token。
fn read_webui_token() -> Option<String> {
    let docker_live = docker_napcat_running();
    let mut docker_paths = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        docker_paths.push(cwd.join("docker/config/webui.json"));
        docker_paths.push(cwd.join("config/webui.json"));
    }
    let mut local_paths = Vec::new();
    if let Some(dir) = dirs::config_dir() {
        local_paths.push(dir.join("qce-tui").join("webui.json"));
    }
    // macOS 补丁版 QQ：NapCat 运行目录固定在 ~/Library/Application Support/QQ/NapCat。
    #[cfg(target_os = "macos")]
    if let Some(home) = dirs::home_dir() {
        local_paths.push(
            home.join("Library/Application Support/QQ/NapCat/config/webui.json"),
        );
    }
    let (first, second) = if docker_live {
        (docker_paths, local_paths)
    } else {
        (local_paths, docker_paths)
    };
    for path in first.into_iter().chain(second) {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(token) = value.get("token").and_then(serde_json::Value::as_str) {
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qce-tui")
        .join("config.json")
}

fn read_config_file() -> serde_json::Value {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn read_local_security_token() -> Option<String> {
    // Docker 容器在线时令牌在容器内的命名卷里，宿主机的 security.json 是陈旧副本。
    if docker_napcat_running() {
        if let Some(token) = read_docker_security_token() {
            return Some(token);
        }
    }
    // 本机 qce-server（含 macOS 补丁版 QQ 内嵌的实例）。
    let path = dirs::home_dir()?
        .join(".qq-chat-exporter")
        .join("security.json");
    let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    value
        .get("accessToken")
        .and_then(serde_json::Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// 从运行中的 `napcat-qce` 容器内读取 API 令牌。
fn read_docker_security_token() -> Option<String> {
    let out = std::process::Command::new("docker")
        .args([
            "exec",
            "napcat-qce",
            "cat",
            "/app/.qq-chat-exporter/security.json",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    value
        .get("accessToken")
        .and_then(serde_json::Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}
