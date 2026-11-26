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

fn help() -> String {
    "qce-tui — QQ Chat Exporter 终端客户端\n\n\
     用法:\n  \
     qce-tui [--host http://127.0.0.1:40653] [--token <token>]\n  \
     qce-tui login [--webui <url>]\n\n\
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

/// 在本机常见位置探测 `NapCat` webui.json（Docker 部署挂载目录优先）。
fn read_webui_token() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let mut candidates = vec![
        cwd.join("docker/config/webui.json"),
        cwd.join("config/webui.json"),
    ];
    if let Some(dir) = dirs::config_dir() {
        candidates.push(dir.join("qce-tui").join("webui.json"));
    }
    for path in candidates {
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
