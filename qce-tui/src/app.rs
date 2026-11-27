use crate::api::QceClient;
use crate::lineedit::LineEdit;
use crate::model::{ChatMessage, Member, Session};
use base64::Engine as _;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// 预览状态：静态图只有 proto；动图按 delay 逐帧轮换 proto。
pub struct Preview {
    pub proto: StatefulProtocol,
    pub title: String,
    /// 帧数 > 1 时视为动画（GIF/APNG）。
    frames: Vec<AnimFrame>,
    idx: usize,
    wait: Duration,
}

impl Preview {
    /// 动画总帧数（<=1 表示静态）。
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// 当前帧下标。
    pub fn idx(&self) -> usize {
        self.idx
    }
}

/// 解码出的一帧动画。
struct AnimFrame {
    img: image::DynamicImage,
    delay: Duration,
}

/// 界面顶层：会话列表 / 聊天页。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Sessions,
    Chat,
}

/// 覆盖层（同一时间只开一个）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    /// 消息输入模式。
    Input,
    /// 图片路径输入。
    ImagePath,
    /// 群成员 @ 选择器。
    MentionPicker,
    /// 图片内联预览。
    Preview,
}

/// 会话列表页签。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Recent,
    Friends,
    Groups,
}

impl Tab {
    pub fn next(self) -> Self {
        match self {
            Self::Recent => Self::Friends,
            Self::Friends => Self::Groups,
            Self::Groups => Self::Recent,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Recent => "recent",
            Self::Friends => "peers",
            Self::Groups => "groups",
        }
    }
}

/// 后台任务回传给 UI 线程的事件。
pub enum AppEvent {
    Sessions(Tab, Result<Vec<Session>, String>),
    Messages(Result<Vec<ChatMessage>, String>),
    Members(Result<Vec<Member>, String>),
    Sent(Result<(), String>),
    ImageStaged(Result<(String, String), String>),
    MediaOpened(Result<String, String>),
    PreviewLoaded(Result<(String, Vec<u8>), String>),
}

const POLL_INTERVAL: Duration = Duration::from_secs(4);
const MAX_MENTIONS: usize = 20;

#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub client: QceClient,
    tx: mpsc::UnboundedSender<AppEvent>,
    rx: mpsc::UnboundedReceiver<AppEvent>,

    pub screen: Screen,
    pub overlay: Overlay,

    // 会话列表
    pub tab: Tab,
    pub sessions: Vec<Session>,
    pub selected: usize,
    pub filter: LineEdit,
    pub filtering: bool,

    // 聊天页
    pub session: Option<Session>,
    pub messages: Vec<ChatMessage>,
    /// 距底部滚动的行数；0 = 跟随最新。
    pub scroll_bottom: usize,
    /// 当前选中的消息下标（j/k/鼠标移动）。
    pub sel_msg: usize,
    /// 每条消息在渲染行中的范围 [start, end)，ui 每帧回填。
    pub msg_ranges: Vec<(usize, usize)>,
    /// 消息区屏幕上内边区域（不含边框），ui 每帧回填，鼠标命中用。
    pub msg_area: Rect,
    /// 本帧实际滚动起点行号，ui 回填，鼠标命中用。
    pub msg_start: usize,
    /// 上次点击（时间, 消息下标），用于双击打开媒体。
    last_click: Option<(Instant, usize)>,
    /// 终端图形协议选择器（启动时探测，不支持则半块渲染）。
    picker: Option<Picker>,
    /// 当前内联预览状态（含动画帧）。
    pub preview: Option<Preview>,

    // 输入区
    pub input: LineEdit,
    pub image: Option<(String, String)>, // (文件名, base64)
    pub image_path: LineEdit,
    pub mentions: Vec<Member>,

    // @ 选择器
    pub members: Vec<Member>,
    pub member_filter: LineEdit,
    pub member_selected: usize,
    pub members_loading: bool,

    pub status: String,
    pub busy: bool,
    pub quit: bool,
    last_poll: Instant,
}

impl App {
    pub fn new(client: QceClient) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            client,
            tx,
            rx,
            screen: Screen::Sessions,
            overlay: Overlay::None,
            tab: Tab::Recent,
            sessions: Vec::new(),
            selected: 0,
            filter: LineEdit::default(),
            filtering: false,
            session: None,
            messages: Vec::new(),
            scroll_bottom: 0,
            sel_msg: 0,
            msg_ranges: Vec::new(),
            msg_area: Rect::default(),
            msg_start: 0,
            last_click: None,
            picker: None,
            preview: None,
            input: LineEdit::default(),
            image: None,
            image_path: LineEdit::default(),
            mentions: Vec::new(),
            members: Vec::new(),
            member_filter: LineEdit::default(),
            member_selected: 0,
            members_loading: false,
            status: "加载会话中…".to_string(),
            busy: false,
            quit: false,
            last_poll: Instant::now(),
        }
    }

    /// 启动后由 main 调用，加载首个页签。
    pub fn request_initial_load(&mut self) {
        self.load_sessions();
    }

    /// main 在进入终端原始模式后注入图形协议选择器。
    pub fn set_picker(&mut self, picker: Picker) {
        self.picker = Some(picker);
    }

    fn spawn<F>(&self, fut: F)
    where
        F: std::future::Future<Output = AppEvent> + Send + 'static,
    {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(fut.await);
        });
    }

    fn load_sessions(&mut self) {
        self.busy = true;
        self.status = format!("加载{}…", self.tab.title());
        let client = self.client.clone();
        let tab = self.tab;
        self.spawn(async move {
            let result = match tab {
                Tab::Recent => client.recent_contacts().await,
                Tab::Friends => client.friends().await,
                Tab::Groups => client.groups().await,
            };
            AppEvent::Sessions(tab, result.map_err(|e| e.to_string()))
        });
    }

    fn open_chat(&mut self) {
        let Some(session) = self
            .filtered_sessions()
            .get(self.selected)
            .map(|s| (*s).clone())
        else {
            return;
        };
        self.session = Some(session);
        self.screen = Screen::Chat;
        self.messages.clear();
        self.scroll_bottom = 0;
        self.sel_msg = 0;
        self.msg_ranges.clear();
        self.refresh_messages();
    }

    fn refresh_messages(&mut self) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let client = self.client.clone();
        self.spawn(async move {
            AppEvent::Messages(
                client
                    .fetch_messages(&session, 1)
                    .await
                    .map_err(|e| e.to_string()),
            )
        });
    }

    fn open_mention_picker(&mut self) {
        let Some(session) = &self.session else { return };
        if session.chat_type != 2 {
            self.status = "@ 提及仅在群聊可用".to_string();
            return;
        }
        self.overlay = Overlay::MentionPicker;
        self.member_filter.clear();
        self.member_selected = 0;
        if self.members.is_empty() && !self.members_loading {
            self.members_loading = true;
            let client = self.client.clone();
            let code = session.peer_uid.clone();
            self.spawn(async move {
                AppEvent::Members(client.group_members(&code).await.map_err(|e| e.to_string()))
            });
        }
    }

    fn stage_image(&mut self) {
        let path = self.image_path.text.trim().to_string();
        if path.is_empty() {
            self.overlay = Overlay::Input;
            return;
        }
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let event = match std::fs::read(&path) {
                Ok(bytes) => {
                    let name = std::path::Path::new(&path)
                        .file_name()
                        .map_or_else(|| "image".to_string(), |n| n.to_string_lossy().into_owned());
                    Ok((
                        name,
                        base64::engine::general_purpose::STANDARD.encode(bytes),
                    ))
                }
                Err(e) => Err(format!("读取文件失败: {e}")),
            };
            let _ = tx.send(AppEvent::ImageStaged(event));
        });
        self.overlay = Overlay::Input;
        self.status = "读取图片中…".to_string();
    }

    fn send(&mut self) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if self.busy {
            return;
        }
        let text = self.input.text.trim().to_string();
        let image = self.image.as_ref().map(|(_, b64)| b64.clone());
        let mentions: Vec<String> = self.mentions.iter().map(|m| m.uin.clone()).collect();
        if text.is_empty() && image.is_none() {
            self.status = "内容为空".to_string();
            return;
        }
        self.busy = true;
        self.status = "发送中…".to_string();
        let client = self.client.clone();
        self.spawn(async move {
            AppEvent::Sent(
                client
                    .send_message(&session, &text, image.as_deref(), &mentions)
                    .await
                    .map_err(|e| e.to_string()),
            )
        });
    }

    /// 当前页签+过滤后的会话（UI 渲染用）。
    pub fn filtered_display(&self) -> Vec<&Session> {
        self.filtered_sessions()
    }

    /// 当前会话显示名。
    pub fn peer_name(&self) -> String {
        self.session
            .as_ref()
            .map(Session::display_name)
            .unwrap_or_default()
    }

    fn filtered_sessions(&self) -> Vec<&Session> {
        let f = self.filter.text.to_lowercase();
        self.sessions
            .iter()
            .filter(|s| {
                f.is_empty()
                    || s.display_name().to_lowercase().contains(&f)
                    || s.peer_uin.contains(&f)
                    || s.peer_uid.to_lowercase().contains(&f)
            })
            .collect()
    }

    pub fn filtered_members(&self) -> Vec<&Member> {
        let f = self.member_filter.text.to_lowercase();
        self.members
            .iter()
            .filter(|m| f.is_empty() || m.name.to_lowercase().contains(&f) || m.uin.contains(&f))
            .collect()
    }

    /// 每个 tick 调用：处理事件 + 定时轮询。
    pub fn tick(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.handle_event(event);
        }
        self.advance_anim();
        if self.screen == Screen::Chat
            && self.overlay == Overlay::None
            && self.last_poll.elapsed() >= POLL_INTERVAL
        {
            self.last_poll = Instant::now();
            self.refresh_messages();
        }
    }

    /// 动图预览：按帧延时推进，重新生成对应帧的渲染协议。
    fn advance_anim(&mut self) {
        const TICK: Duration = Duration::from_millis(100);
        let Some(pv) = &mut self.preview else {
            return;
        };
        if pv.frames.len() < 2 {
            return;
        }
        if pv.wait > TICK {
            pv.wait -= TICK;
            return;
        }
        pv.idx = (pv.idx + 1) % pv.frames.len();
        pv.wait = pv.frames[pv.idx].delay;
        if let Some(picker) = &self.picker {
            pv.proto = picker.new_resize_protocol(pv.frames[pv.idx].img.clone());
        }
    }

    /// 解码预览：多帧 GIF/APNG 走动画，否则静态图。
    fn make_preview(&self, name: String, bytes: &[u8]) -> Result<Preview, String> {
        let Some(picker) = &self.picker else {
            return Err("图形协议未初始化".to_string());
        };
        let frames = decode_anim_frames(bytes);
        if frames.len() > 1 {
            let proto = picker.new_resize_protocol(frames[0].img.clone());
            return Ok(Preview {
                proto,
                title: name,
                frames,
                idx: 0,
                wait: Duration::ZERO,
            });
        }
        let img = image::load_from_memory(bytes).map_err(|_| "不是可预览的图片格式".to_string())?;
        Ok(Preview {
            proto: picker.new_resize_protocol(img),
            title: name,
            frames: Vec::new(),
            idx: 0,
            wait: Duration::ZERO,
        })
    }

    fn handle_event(&mut self, event: AppEvent) {
        self.busy = false;
        match event {
            AppEvent::Sessions(tab, result) => match result {
                Ok(list) => {
                    if self.tab == tab {
                        self.sessions = list;
                        self.selected = self
                            .selected
                            .min(self.filtered_sessions().len().saturating_sub(1));
                        self.status = format!("{} {} 个会话", tab.title(), self.sessions.len());
                    }
                }
                Err(e) => self.status = format!("加载失败: {e}"),
            },
            AppEvent::Messages(result) => match result {
                Ok(mut list) => {
                    list.reverse(); // 服务端最新在前 → 展示最旧在前
                                    // 选中在旧列表尾部 → 跟随新消息保持在尾部；
                                    // 否则按 msg_id 找回原位，避免轮询把选区拉回底部。
                    let at_tail = self.sel_msg + 1 >= self.messages.len();
                    let sel_id = (!at_tail)
                        .then(|| self.messages.get(self.sel_msg).map(|m| m.msg_id.clone()))
                        .flatten()
                        .filter(|id| !id.is_empty());
                    self.messages = list;
                    self.sel_msg = if at_tail {
                        self.messages.len().saturating_sub(1)
                    } else {
                        sel_id
                            .and_then(|id| self.messages.iter().position(|m| m.msg_id == id))
                            .unwrap_or_else(|| {
                                self.sel_msg.min(self.messages.len().saturating_sub(1))
                            })
                    };
                    self.status = format!("{} 条消息", self.messages.len());
                }
                Err(e) => self.status = format!("拉取失败: {e}"),
            },
            AppEvent::Members(result) => {
                self.members_loading = false;
                match result {
                    Ok(list) => {
                        self.status = format!("{} 个成员", list.len());
                        self.members = list;
                    }
                    Err(e) => self.status = format!("成员加载失败: {e}"),
                }
            }
            AppEvent::Sent(result) => match result {
                Ok(()) => {
                    self.input.clear();
                    self.image = None;
                    self.mentions.clear();
                    self.status = "已发送 ✓".to_string();
                    self.refresh_messages();
                }
                Err(e) => self.status = format!("发送失败: {e}"),
            },
            AppEvent::ImageStaged(result) => match result {
                Ok((name, b64)) => {
                    self.image = Some((name.clone(), b64));
                    self.status = format!("已附加图片 {name}（Enter 发送 / Esc 取消）");
                }
                Err(e) => self.status = e,
            },
            AppEvent::MediaOpened(result) => match result {
                Ok(msg) => self.status = msg,
                Err(e) => self.status = e,
            },
            AppEvent::PreviewLoaded(result) => match result {
                Ok((name, bytes)) => match self.make_preview(name.clone(), &bytes) {
                    Ok(pv) => {
                        self.overlay = Overlay::Preview;
                        self.status = format!("预览 {name}");
                        self.preview = Some(pv);
                    }
                    Err(e) => self.status = e,
                },
                Err(e) => self.status = format!("加载媒体失败: {e}"),
            },
        }
    }

    /// 用系统默认程序打开路径或 URL。
    fn system_open(target: &str) -> Result<(), String> {
        let result = if cfg!(target_os = "macos") {
            std::process::Command::new("open").arg(target).status()
        } else if cfg!(target_os = "windows") {
            std::process::Command::new("cmd")
                .args(["/c", "start", "", target])
                .status()
        } else {
            std::process::Command::new("xdg-open").arg(target).status()
        };
        match result {
            Ok(st) if st.success() => Ok(()),
            Ok(st) => Err(format!("打开失败（退出码 {st}）")),
            Err(e) => Err(format!("无法启动系统打开器: {e}")),
        }
    }

    /// 内联预览选中消息的图片（下载后经终端协议渲染）。
    fn preview_media(&mut self, idx: usize) {
        let (Some(session), Some(msg)) = (self.session.clone(), self.messages.get(idx)) else {
            return;
        };
        let Some(media) = msg.media.first().cloned() else {
            self.status = "该消息没有可预览的媒体".to_string();
            return;
        };
        if media.local_path.is_none() {
            self.status = "媒体无本地缓存，按 O 尝试远程打开".to_string();
            return;
        }
        self.status = format!("加载 {} …", media.file_name);
        let client = self.client.clone();
        self.spawn(async move {
            AppEvent::PreviewLoaded(
                client
                    .fetch_media(&media, &session)
                    .await
                    .map(|(b, _)| (media.file_name.clone(), b))
                    .map_err(|e| e.to_string()),
            )
        });
    }

    /// 下载选中消息媒体到缓存目录并用系统看图工具打开；
    /// 无本地缓存时回退打开远程 URL。
    fn open_media(&mut self, idx: usize) {
        let (Some(session), Some(msg)) = (self.session.clone(), self.messages.get(idx)) else {
            return;
        };
        let Some(media) = msg.media.first().cloned() else {
            self.status = "该消息没有可打开的媒体".to_string();
            return;
        };
        self.status = format!("下载 {} …", media.file_name);
        let client = self.client.clone();
        self.spawn(async move {
            let result: Result<String, String> = async {
                let remote = media.remote_url.clone().filter(|u| u.starts_with("http"));
                if media.local_path.is_some() {
                    match client.fetch_media(&media, &session).await {
                        Ok((bytes, ext)) => {
                            let dir = dirs::cache_dir()
                                .unwrap_or_else(std::env::temp_dir)
                                .join("qce-tui");
                            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                            let stem = if media.element_id.is_empty() {
                                media.msg_id.clone()
                            } else {
                                media.element_id.clone()
                            };
                            let file = dir.join(format!("{stem}.{ext}"));
                            std::fs::write(&file, &bytes).map_err(|e| e.to_string())?;
                            Self::system_open(&file.to_string_lossy())?;
                            Ok(format!("已打开 {}", media.file_name))
                        }
                        // 旧消息媒体可能已过 CDN 有效期无法回源，退回远程链接试试。
                        Err(e) => match remote {
                            Some(url) => {
                                Self::system_open(&url)?;
                                Ok("本地缓存不可用，已在浏览器打开远程媒体".to_string())
                            }
                            None => Err(format!("{e}（该消息媒体可能已过期）")),
                        },
                    }
                } else if let Some(url) = remote {
                    Self::system_open(&url)?;
                    Ok("已在浏览器打开远程媒体".to_string())
                } else {
                    Err("媒体没有可用来源".to_string())
                }
            }
            .await;
            AppEvent::MediaOpened(result)
        });
    }

    /// 转发表情包：把选中消息的图片/动图下载后附加为待发图片。
    fn stage_media(&mut self, idx: usize) {
        let (Some(session), Some(msg)) = (self.session.clone(), self.messages.get(idx)) else {
            return;
        };
        let Some(media) = msg.media.first().cloned() else {
            self.status = "该消息没有可转发的媒体".to_string();
            return;
        };
        if media.local_path.is_none() {
            self.status = "媒体无本地缓存，无法转发".to_string();
            return;
        }
        self.status = format!("读取 {} …", media.file_name);
        let client = self.client.clone();
        self.spawn(async move {
            AppEvent::ImageStaged(
                client
                    .fetch_media(&media, &session)
                    .await
                    .map(|(b, _)| {
                        (
                            media.file_name.clone(),
                            base64::engine::general_purpose::STANDARD.encode(b),
                        )
                    })
                    .map_err(|e| format!("读取媒体失败: {e}")),
            )
        });
    }

    /// 复制选中消息媒体的链接到剪贴板。
    fn copy_media_link(&mut self, idx: usize) {
        let Some(msg) = self.messages.get(idx) else {
            return;
        };
        let Some(media) = msg.media.first() else {
            self.status = "该消息没有可复制链接的媒体".to_string();
            return;
        };
        let url = self
            .session
            .as_ref()
            .and_then(|s| self.client.media_url(media, s))
            .or_else(|| media.remote_url.clone());
        let Some(url) = url else {
            self.status = "媒体没有可用链接".to_string();
            return;
        };
        let copied = if cfg!(target_os = "macos") {
            std::process::Command::new("pbcopy")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut c| {
                    use std::io::Write as _;
                    c.stdin.take().unwrap().write_all(url.as_bytes())?;
                    c.wait()
                })
                .map(|s| s.success())
        } else if cfg!(target_os = "windows") {
            std::process::Command::new("cmd")
                .args(["/c", "clip"])
                .stdin(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut c| {
                    use std::io::Write as _;
                    c.stdin.take().unwrap().write_all(url.as_bytes())?;
                    c.wait()
                })
                .map(|s| s.success())
        } else {
            std::process::Command::new("xclip")
                .args(["-selection", "clipboard"])
                .stdin(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut c| {
                    use std::io::Write as _;
                    c.stdin.take().unwrap().write_all(url.as_bytes())?;
                    c.wait()
                })
                .map(|s| s.success())
        };
        self.status = match copied {
            Ok(true) => "链接已复制".to_string(),
            _ => "复制失败（需要 pbcopy/xclip/clip）".to_string(),
        };
    }

    /// 鼠标事件：点击选中消息（双击打开媒体）、滚轮滚动。
    pub fn on_mouse(&mut self, ev: MouseEvent) {
        if self.screen != Screen::Chat || self.overlay != Overlay::None {
            return;
        }
        let area = self.msg_area;
        let inside = ev.column >= area.x
            && ev.column < area.x + area.width
            && ev.row >= area.y
            && ev.row < area.y + area.height;
        match ev.kind {
            MouseEventKind::ScrollUp if inside => {
                self.scroll_bottom = self.scroll_bottom.saturating_add(3);
            }
            MouseEventKind::ScrollDown if inside => {
                self.scroll_bottom = self.scroll_bottom.saturating_sub(3);
            }
            MouseEventKind::Down(MouseButton::Left) if inside => {
                let line = self.msg_start + usize::from(ev.row - area.y);
                if let Some(idx) = self
                    .msg_ranges
                    .iter()
                    .position(|(s, e)| line >= *s && line < *e)
                {
                    let now = Instant::now();
                    let double = self
                        .last_click
                        .is_some_and(|(t, i)| i == idx && now.duration_since(t).as_millis() < 400);
                    self.sel_msg = idx;
                    if double {
                        self.last_click = None;
                        self.preview_media(idx);
                    } else {
                        self.last_click = Some((now, idx));
                    }
                }
            }
            _ => {}
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        match self.overlay {
            Overlay::Preview => {
                self.overlay = Overlay::None;
                self.preview = None;
            }
            Overlay::Input => self.on_key_input(key),
            Overlay::ImagePath => self.on_key_image_path(key),
            Overlay::MentionPicker => self.on_key_mentions(key),
            Overlay::None => match self.screen {
                Screen::Sessions => self.on_key_sessions(key),
                Screen::Chat => self.on_key_chat(key),
            },
        }
    }

    fn on_key_sessions(&mut self, key: KeyEvent) {
        if self.filtering {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => self.filtering = false,
                _ => {
                    if Self::edit_key(&mut self.filter, key) == EditEffect::Changed {
                        self.selected = 0;
                    }
                }
            }
            return;
        }
        let len = self.filtered_sessions().len();
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('j') | KeyCode::Down => {
                self.selected = (self.selected + 1).min(len.saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Enter => self.open_chat(),
            KeyCode::Tab | KeyCode::BackTab => {
                self.tab = self.tab.next();
                self.selected = 0;
                self.filter.clear();
                self.load_sessions();
            }
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Char('r') => self.load_sessions(),
            _ => {}
        }
    }

    fn on_key_chat(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.screen = Screen::Sessions;
                self.session = None;
                self.input.clear();
                self.image = None;
                self.mentions.clear();
                self.status.clear();
            }
            KeyCode::Char('i') | KeyCode::Enter => self.overlay = Overlay::Input,
            KeyCode::Char('a') => self.open_mention_picker(),
            KeyCode::Char('p') => {
                self.overlay = Overlay::ImagePath;
                self.image_path.clear();
            }
            KeyCode::Char('r') => self.refresh_messages(),
            KeyCode::Char('j') | KeyCode::Down => {
                self.sel_msg = (self.sel_msg + 1).min(self.messages.len().saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.sel_msg = self.sel_msg.saturating_sub(1);
            }
            KeyCode::Char('o') => self.preview_media(self.sel_msg),
            KeyCode::Char('O') => self.open_media(self.sel_msg),
            KeyCode::Char('y') => self.copy_media_link(self.sel_msg),
            KeyCode::Char('s') => self.stage_media(self.sel_msg),
            KeyCode::PageDown => {
                self.scroll_bottom = self.scroll_bottom.saturating_sub(10);
            }
            KeyCode::PageUp => {
                self.scroll_bottom += 10;
            }
            KeyCode::Char('G') => {
                self.scroll_bottom = 0;
                self.sel_msg = self.messages.len().saturating_sub(1);
            }
            _ => {}
        }
    }

    fn on_key_input(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.overlay = Overlay::None;
            }
            KeyCode::Enter => {
                // 发送后保持在输入模式，可连续打字发送（Esc 才退出）。
                self.send();
            }
            _ => {
                Self::edit_key(&mut self.input, key);
            }
        }
    }

    fn on_key_image_path(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::Input,
            KeyCode::Enter => self.stage_image(),
            _ => {
                Self::edit_key(&mut self.image_path, key);
            }
        }
    }

    fn on_key_mentions(&mut self, key: KeyEvent) {
        let len = self.filtered_members().len();
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::Input,
            KeyCode::Up => {
                self.member_selected = self.member_selected.saturating_sub(1);
            }
            KeyCode::Down => {
                self.member_selected = (self.member_selected + 1).min(len.saturating_sub(1));
            }
            KeyCode::Enter => {
                if let Some(member) = self
                    .filtered_members()
                    .get(self.member_selected)
                    .map(|m| (*m).clone())
                {
                    if self.mentions.len() >= MAX_MENTIONS {
                        self.status = format!("最多 @ {MAX_MENTIONS} 人");
                    } else if !self.mentions.iter().any(|m| m.uin == member.uin) {
                        self.mentions.push(member);
                    }
                    self.overlay = Overlay::Input;
                }
            }
            _ => {
                if Self::edit_key(&mut self.member_filter, key) == EditEffect::Changed {
                    self.member_selected = 0;
                }
            }
        }
    }

    /// 单行编辑通用按键：光标移动、删除、插入。
    /// 返回编辑效果，调用方据此决定是否联动（如文本变化时重置列表选中）。
    fn edit_key(edit: &mut LineEdit, key: KeyEvent) -> EditEffect {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return match key.code {
                KeyCode::Char('a') => {
                    edit.home();
                    EditEffect::Moved
                }
                KeyCode::Char('e') => {
                    edit.end();
                    EditEffect::Moved
                }
                KeyCode::Char('b') | KeyCode::Left => {
                    edit.word_left();
                    EditEffect::Moved
                }
                KeyCode::Char('f') | KeyCode::Right => {
                    edit.word_right();
                    EditEffect::Moved
                }
                KeyCode::Char('u') => {
                    edit.kill_before_cursor();
                    EditEffect::Changed
                }
                KeyCode::Char('k') => {
                    edit.kill_after_cursor();
                    EditEffect::Changed
                }
                KeyCode::Char('w') | KeyCode::Backspace => {
                    edit.delete_word_back();
                    EditEffect::Changed
                }
                _ => EditEffect::Ignored,
            };
        }
        match key.code {
            KeyCode::Left => {
                edit.move_left();
                EditEffect::Moved
            }
            KeyCode::Right => {
                edit.move_right();
                EditEffect::Moved
            }
            KeyCode::Home => {
                edit.home();
                EditEffect::Moved
            }
            KeyCode::End => {
                edit.end();
                EditEffect::Moved
            }
            KeyCode::Backspace => {
                edit.backspace();
                EditEffect::Changed
            }
            KeyCode::Delete => {
                edit.delete();
                EditEffect::Changed
            }
            KeyCode::Char(c) => {
                edit.insert(c);
                EditEffect::Changed
            }
            _ => EditEffect::Ignored,
        }
    }
}

/// `edit_key` 的效果：移动光标不改变文本，插入/删除才算内容变化。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditEffect {
    Ignored,
    Moved,
    Changed,
}

/// 解码 GIF/APNG 动图帧；静态图或解码失败返回空 Vec。
fn decode_anim_frames(bytes: &[u8]) -> Vec<AnimFrame> {
    use image::AnimationDecoder as _;
    let cursor = || std::io::BufReader::new(std::io::Cursor::new(bytes));
    let frames = if bytes.starts_with(b"GIF8") {
        image::codecs::gif::GifDecoder::new(cursor()).and_then(|d| d.into_frames().collect_frames())
    } else if bytes.starts_with(b"\x89PNG") {
        image::codecs::png::PngDecoder::new(cursor())
            .and_then(image::codecs::png::PngDecoder::apng)
            .and_then(|d| d.into_frames().collect_frames())
    } else {
        return Vec::new();
    };
    let Ok(frames) = frames else {
        return Vec::new();
    };
    frames
        .into_iter()
        .map(|f| {
            let (n, d) = f.delay().numer_denom_ms();
            // 比率单位即毫秒：delay = n/d ms
            let ms = u64::from(n) / u64::from(d.max(1));
            AnimFrame {
                img: image::DynamicImage::ImageRgba8(f.into_buffer()),
                delay: Duration::from_millis(ms.clamp(40, 30_000)),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str) -> ChatMessage {
        ChatMessage {
            msg_id: id.to_string(),
            time: 0,
            sender: "u".to_string(),
            is_self: false,
            body: "x".to_string(),
            media: Vec::new(),
        }
    }

    fn app() -> App {
        App::new(QceClient::new("http://127.0.0.1:1", "t"))
    }

    #[test]
    fn sel_follows_tail_when_at_tail() {
        let mut a = app();
        a.messages = vec![msg("a"), msg("b"), msg("c")];
        a.sel_msg = 2; // 尾部
        a.handle_event(AppEvent::Messages(Ok(vec![
            msg("d"),
            msg("c"),
            msg("b"),
            msg("a"),
        ])));
        // 列表 reverse 后 [a,b,c,d]，仍在尾部 → 跟到新尾 d
        assert_eq!(a.sel_msg, 3);
        assert_eq!(a.messages[3].msg_id, "d");
    }

    #[test]
    fn sel_preserved_by_msg_id_when_scrolled_up() {
        let mut a = app();
        a.messages = vec![msg("a"), msg("b"), msg("c")];
        a.sel_msg = 1; // 上移选中 b
        a.handle_event(AppEvent::Messages(Ok(vec![
            msg("d"),
            msg("c"),
            msg("b"),
            msg("a"),
        ])));
        assert_eq!(a.sel_msg, 1);
        assert_eq!(a.messages[1].msg_id, "b");
    }

    #[test]
    fn sel_clamped_when_message_gone() {
        let mut a = app();
        a.messages = vec![msg("a"), msg("b"), msg("c")];
        a.sel_msg = 0; // 选中 a，刷新后 a 掉出窗口
        a.handle_event(AppEvent::Messages(Ok(vec![msg("c"), msg("b")])));
        assert_eq!(a.sel_msg, 0);
        assert_eq!(a.messages[0].msg_id, "b");
    }
}
