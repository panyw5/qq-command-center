//! 统一状态管理：`qce status` / `qce clean` / `qce restart`。
//!
//! 三种部署形态共用 6099/40653 端口，任一时刻只有一方在线；
//! qce-server 的父进程是 QQ 主进程，父进程死后变孤儿（ppid=1）即为陈旧实例。

use serde_json::Value;
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::Duration;

struct ProcInfo {
    pid: u32,
    ppid: u32,
    command: String,
}

/// 列出匹配子串的进程（pid, ppid, command）。
fn find_procs(pattern: &str) -> Vec<ProcInfo> {
    let Ok(out) = Command::new("pgrep").args(["-fl", pattern]).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let pid: u32 = line.split_whitespace().next()?.parse().ok()?;
            let ppid = Command::new("ps")
                .args(["-o", "ppid=", "-p", &pid.to_string()])
                .output()
                .ok()
                .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
                .unwrap_or(0);
            Some(ProcInfo {
                pid,
                ppid,
                command: line.to_string(),
            })
        })
        .collect()
}

fn tcp_open(port: u16) -> bool {
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

fn qq_app_procs() -> Vec<ProcInfo> {
    find_procs("Contents/MacOS/QQ")
        .into_iter()
        .filter(|p| !p.command.contains("QQ Helper"))
        .collect()
}

fn qce_server_procs() -> Vec<ProcInfo> {
    find_procs("qce-server")
        .into_iter()
        .filter(|p| p.command.contains("qce-server"))
        .collect()
}

fn docker_running() -> bool {
    Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", "napcat-qce"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// 孤儿 qce-server（父进程已死）视为陈旧实例。
pub fn clean() {
    let orphans: Vec<_> = qce_server_procs()
        .into_iter()
        .filter(|p| p.ppid == 1)
        .collect();
    if orphans.is_empty() {
        println!("没有需要清理的陈旧 qce-server 进程。");
        return;
    }
    for p in &orphans {
        let ok = Command::new("kill")
            .arg(p.pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        println!(
            "{} qce-server pid={} (孤儿进程)",
            if ok { "已终止" } else { "终止失败" },
            p.pid
        );
    }
}

/// `qce status`：一张表看清"哪活着、谁登着、有没有僵尸"。
pub async fn status(host: &str, token: Option<&str>) {
    println!("== 部署 ==");
    let docker = docker_running();
    let qqs = qq_app_procs();
    if docker {
        println!("  Docker napcat-qce : 运行中");
    }
    if qqs.is_empty() && !docker {
        println!("  没有运行中的 QQ/NapCat 实例");
    }
    for p in &qqs {
        println!("  QQ 实例           : pid={} {}", p.pid, short_app(&p.command));
    }

    println!("\n== 端口 ==");
    println!("  6099  NapCat WebUI: {}", if tcp_open(6099) { "✓ 在听" } else { "✗ 未监听" });
    println!("  40653 QCE API     : {}", if tcp_open(40653) { "✓ 在听" } else { "✗ 未监听" });

    println!("\n== qce-server 进程 ==");
    let servers = qce_server_procs();
    if servers.is_empty() {
        println!("  无（若 QQ 已登录仍无，说明插件未拉起或端口被占）");
    }
    for p in &servers {
        let tag = if p.ppid == 1 { "⚠ 孤儿（陈旧实例，建议 qce clean）" } else { "正常" };
        println!("  pid={} ppid={} {}", p.pid, p.ppid, tag);
    }

    println!("\n== QQ 登录态 ==");
    if !tcp_open(40653) {
        println!("  QCE 未监听，无法探测（先让服务起来）");
    } else if let Some(t) = token {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        let url = format!("{}/api/system/info", host.trim_end_matches('/'));
        match client.get(&url).bearer_auth(t).send().await {
            Ok(resp) => match resp.json::<Value>().await {
                Ok(v) => {
                    let data = v.get("data").cloned().unwrap_or(v);
                    let napcat = data.get("napcat").cloned().unwrap_or(Value::Null);
                    let online = napcat.get("online").and_then(Value::as_bool).unwrap_or(false);
                    let self_info = napcat.get("selfInfo").cloned().unwrap_or(Value::Null);
                    let uin = self_info
                        .get("uin")
                        .and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_u64().map(|n| n.to_string())))
                        .unwrap_or_default();
                    let nick = self_info
                        .get("nick")
                        .or_else(|| self_info.get("nickname"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if online && !uin.is_empty() {
                        println!("  ✓ 已登录: {nick} ({uin})");
                    } else {
                        println!("  ✗ 未登录（40653 有响应但会话离线——若是刚重启，等自动登录完成）");
                    }
                }
                Err(_) => println!("  响应不可解析（可能是陈旧 qce-server 在占端口，跑 qce clean）"),
            },
            Err(e) => println!("  探测失败: {e}"),
        }
    } else {
        println!("  无 token，跳过登录态探测");
    }
}

/// `qce restart`：退出运行中的实例 → 清孤儿 → 拉起 QQ-NapCat 应用 → 等服务就绪。
pub async fn restart(host: &str, token: Option<&str>) {
    // 1. 退出在跑的 QQ-NapCat 实例（SIGTERM 让它正常收尾）
    for p in qq_app_procs() {
        if p.command.contains("QQ-NapCat") {
            let _ = Command::new("kill").arg(p.pid.to_string()).status();
            println!("已退出运行中的实例 pid={}", p.pid);
        }
    }
    if !qq_app_procs().is_empty() {
        std::thread::sleep(Duration::from_secs(3));
    }
    clean();
    #[cfg(target_os = "macos")]
    {
        let app = find_napcat_app();
        match app {
            Some(path) => {
                println!("启动 {path} ...");
                let _ = Command::new("open").arg(&path).status();
            }
            None => println!("未找到 QQ-NapCat 应用，跳过启动。"),
        }
    }
    #[cfg(not(target_os = "macos"))]
    println!("restart 目前只在 macOS 补丁版场景实现；Docker 用 docker compose restart。");

    // 等 40653 起来（最多 40s）
    for _ in 0..40 {
        if tcp_open(40653) {
            println!("QCE 已就绪。");
            status(host, token).await;
            return;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    println!("等待超时，40653 仍未监听——看 QQ 是否已登录。");
}

#[cfg(target_os = "macos")]
fn find_napcat_app() -> Option<String> {
    let home = dirs::home_dir()?;
    let dir = home.join("Applications");
    let mut best: Option<(String, u32)> = None;
    for entry in fs_entries(&dir) {
        if entry.contains("QQ-NapCat") && entry.ends_with(".app") {
            // 版本号大的优先（QQ-NapCat-704.app > QQ-NapCat.app）
            let ver = entry
                .trim_end_matches(".app")
                .rsplit('-')
                .next()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            if best.as_ref().map(|(_, v)| ver >= *v).unwrap_or(true) {
                best = Some((entry, ver));
            }
        }
    }
    best.map(|(name, _)| dir.join(name).to_string_lossy().to_string())
}

#[cfg(target_os = "macos")]
fn fs_entries(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn short_app(command: &str) -> &str {
    command
        .split("Contents/MacOS")
        .next()
        .and_then(|s| s.rsplit('/').nth(1))
        .unwrap_or(command)
        .trim_start_matches(|c: char| c.is_numeric() || c.is_whitespace())
}
