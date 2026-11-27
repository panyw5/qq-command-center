mod api;
mod app;
mod config;
mod model;
mod ui;

use api::QceClient;
use app::App;
use crossterm::cursor::SetCursorStyle;
use crossterm::event::{self, Event};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use ratatui_image::picker::Picker;
use std::io;
use std::time::Duration;

const TICK: Duration = Duration::from_millis(100);

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let sub = args.first().map(String::as_str);
    let rest = if matches!(sub, Some("tui") | Some("login") | Some("export")) {
        &args[1..]
    } else {
        &args[..]
    };

    // `qce login`：打开 NapCat WebUI 登录页（扫码重登入口）。
    if sub == Some("login") {
        match config::webui_login_url(rest) {
            Ok(url) => match open_in_browser(&url) {
                Ok(()) => println!("已在浏览器打开 NapCat WebUI: {url}"),
                Err(e) => {
                    eprintln!("无法自动打开浏览器 ({e})，请手动访问:\n{url}");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
        return;
    }

    // `qce export`：打印 QCE Web 界面的 token 与访问网址；`--open` 直接打开一键登录页。
    if sub == Some("export") {
        let open = rest.iter().any(|a| a == "--open" || a == "-o");
        let filtered: Vec<String> = rest
            .iter()
            .filter(|a| a.as_str() != "--open" && a.as_str() != "-o")
            .cloned()
            .collect();
        match config::resolve(&filtered) {
            Ok(cfg) => {
                let auth = format!("{}/qce/auth?token={}", cfg.host, cfg.token);
                println!("Token:    {}", cfg.token);
                println!("一键登录: {auth}");
                println!("界面地址: {}/qce", cfg.host);
                if open {
                    if let Err(e) = open_in_browser(&auth) {
                        eprintln!("无法自动打开浏览器 ({e})");
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
        return;
    }

    let cfg = match config::resolve(rest) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };

    let client = QceClient::new(&cfg.host, &cfg.token);
    let mut app = App::new(client);
    app_load_first(&mut app);

    if let Err(e) = run(&mut app) {
        restore_terminal();
        eprintln!("TUI 运行错误: {e}");
        std::process::exit(1);
    }
    restore_terminal();
}

fn app_load_first(app: &mut App) {
    // 触发首次会话加载：直接借用 on_key 里 'r' 的路径太重，
    // 这里给一个公开的入口语义一致的调用。
    app.request_initial_load();
}

fn open_in_browser(url: &str) -> io::Result<()> {
    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).status()?
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .status()?
    } else {
        std::process::Command::new("xdg-open").arg(url).status()?
    };
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("退出码 {status}")))
    }
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        SetCursorStyle::DefaultUserShape,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
}

fn run(app: &mut App) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        SetCursorStyle::BlinkingBar
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // 图形协议选择：不做 stdin 探测（会污染键盘输入流），靠 env 识别
    // tmux/iTerm2，其他终端一律用半块字符渲染（真彩色下显示效果也足够好）。
    let picker = Picker::from_fontsize((8, 16));
    app.set_picker(picker);

    while !app.quit {
        terminal.draw(|f| ui::draw(f, app))?;
        app.tick();

        if event::poll(TICK)? {
            match event::read()? {
                // Windows 上按下/释放都会发事件，只处理按下。
                Event::Key(key) if key.kind == crossterm::event::KeyEventKind::Press => {
                    app.on_key(key);
                }
                Event::Mouse(mouse) => app.on_mouse(mouse),
                _ => {}
            }
        }
    }
    Ok(())
}
