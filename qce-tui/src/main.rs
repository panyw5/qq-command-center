mod api;
mod app;
mod config;
mod model;
mod ui;

use api::QceClient;
use app::App;
use crossterm::event::{self, Event};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::time::Duration;

const TICK: Duration = Duration::from_millis(100);

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match config::resolve(&args) {
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

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
}

fn run(app: &mut App) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

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
