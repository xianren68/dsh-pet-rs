//! 环境事件规则：`assets/events.json` 的结构、加载与模板渲染。
//!
//! 这里**只定义字段和默认值**，所有动作名、气泡文案、阈值都放在 JSON 里。
//! 缺字段时用字段默认值；缺某个事件键就等于关闭该事件；整个文件缺失时
//! 桌宠仍然正常工作，只是不再响应环境事件。

use std::collections::HashMap;

use serde::Deserialize;

use crate::behavior::strip_line_comments;

/// 代码认识的系统事件键。新增传感器时才需要改这个枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKey {
    BatteryLow,
    BatteryCritical,
    BatteryCharging,
    BatteryFull,
    NetworkDown,
    NetworkUp,
    DownloadStart,
    DownloadDone,
    DownloadFailed,
    IdleSleep,
    IdleWake,
    SessionLock,
    SessionUnlock,
    Resume,
    Greeting,
}

impl EventKey {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKey::BatteryLow => "batteryLow",
            EventKey::BatteryCritical => "batteryCritical",
            EventKey::BatteryCharging => "batteryCharging",
            EventKey::BatteryFull => "batteryFull",
            EventKey::NetworkDown => "networkDown",
            EventKey::NetworkUp => "networkUp",
            EventKey::DownloadStart => "downloadStart",
            EventKey::DownloadDone => "downloadDone",
            EventKey::DownloadFailed => "downloadFailed",
            EventKey::IdleSleep => "idleSleep",
            EventKey::IdleWake => "idleWake",
            EventKey::SessionLock => "sessionLock",
            EventKey::SessionUnlock => "sessionUnlock",
            EventKey::Resume => "resume",
            EventKey::Greeting => "greeting",
        }
    }
}

/// 单个事件的规则。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EventRule {
    /// 显式 false 可临时关掉这一条，不用删配置。
    pub enabled: Option<bool>,
    /// 候选动作，随机抽一个；空数组 = 只弹气泡。
    pub actions: Vec<String>,
    /// 气泡模板，支持 `{pct}` / `{name}` / `{size}` / `{time}`。
    pub bubble: Option<String>,
    /// 数字越大越先播。
    pub priority: Option<u8>,
    /// 同一事件两次触发的最小间隔（秒）。
    pub cooldown_sec: Option<u32>,
    /// true 时可以在当前动画播到一半时切进来。
    pub interrupt: Option<bool>,
    /// 动作播完从头循环，保持这个状态，直到收到对应的“退出”事件。
    pub hold: Option<bool>,
    /// 带文字的动作在朝右时先翻回朝左。
    pub no_mirror: Option<bool>,
}

impl EventRule {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
    pub fn priority(&self) -> u8 {
        self.priority.unwrap_or(50)
    }
    pub fn cooldown_sec(&self) -> u32 {
        self.cooldown_sec.unwrap_or(60)
    }
    pub fn interrupt(&self) -> bool {
        self.interrupt.unwrap_or(false)
    }
    pub fn hold(&self) -> bool {
        self.hold.unwrap_or(false)
    }
    pub fn no_mirror(&self) -> bool {
        self.no_mirror.unwrap_or(false)
    }
}

/// 气泡外观。字段都是可选的，缺省时保持 v0.2 原来的白字深灰底。
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BubbleStyle {
    /// 字体名；系统没有这个字体时 GDI 会自动回退。
    pub font_family: String,
    /// 96 DPI 下的字号（像素），实际渲染会按 DPI 缩放。
    pub font_size: u32,
    /// GDI 字重：400 常规，700 粗体。
    pub font_weight: u32,
    /// 文字颜色，`#RRGGBB`。
    pub text_color: String,
    /// 背景颜色，`#RRGGBB`。
    pub background_color: String,
    /// 描边颜色，`#RRGGBB`。浅色底必须靠描边才在浅色桌面上有轮廓。
    pub border_color: String,
    /// 描边宽度（96 DPI 像素），0 = 不描边。
    pub border_width: i32,
    /// 整体不透明度，0–255。
    pub alpha: u32,
    /// 圆角半径，0 = 直角。
    pub corner_radius: i32,
    /// 文字与边缘的内边距。
    pub padding: i32,
    /// 气泡最大宽度。
    pub max_width: i32,
    /// 气泡显示时长（毫秒）。
    pub duration_ms: u32,
}

impl Default for BubbleStyle {
    fn default() -> Self {
        Self {
            font_family: "Microsoft YaHei UI".to_string(),
            font_size: 16,
            font_weight: 400,
            text_color: "#FFFFFF".to_string(),
            background_color: "#2B2B2B".to_string(),
            border_color: "#111111".to_string(),
            border_width: 0,
            alpha: 240,
            corner_radius: 7,
            padding: 12,
            max_width: 320,
            duration_ms: 4_000,
        }
    }
}

impl BubbleStyle {
    pub fn duration_ms(&self) -> u32 {
        self.duration_ms.clamp(500, 60_000)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BatteryCfg {
    pub low_pct: u8,
    pub critical_pct: u8,
    pub poll_sec: u32,
}

impl Default for BatteryCfg {
    fn default() -> Self {
        Self {
            low_pct: 20,
            critical_pct: 10,
            poll_sec: 60,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NetworkCfg {
    pub poll_sec: u32,
    /// 连续多少次轮询结果一致才认，防抖。
    pub confirm_ticks: u32,
}

impl Default for NetworkCfg {
    fn default() -> Self {
        Self {
            poll_sec: 15,
            confirm_ticks: 2,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IdleCfg {
    pub sleep_sec: u32,
    pub wake_sec: u32,
}

impl Default for IdleCfg {
    fn default() -> Self {
        Self {
            sleep_sec: 600,
            wake_sec: 5,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DownloadCfg {
    /// 空 = 自动找系统“下载”文件夹。
    pub dir: String,
    pub poll_sec: u32,
    pub temp_exts: Vec<String>,
}

fn default_temp_exts() -> Vec<String> {
    [
        ".crdownload",
        ".part",
        ".opdownload",
        ".download",
        ".!qb",
        ".aria2",
        ".td",
        ".xltd",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

impl Default for DownloadCfg {
    fn default() -> Self {
        Self {
            dir: String::new(),
            poll_sec: 2,
            temp_exts: default_temp_exts(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FullscreenCfg {
    pub poll_sec: u32,
}

impl Default for FullscreenCfg {
    fn default() -> Self {
        Self { poll_sec: 2 }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SensorsConfig {
    pub battery: BatteryCfg,
    pub network: NetworkCfg,
    pub idle: IdleCfg,
    pub download: DownloadCfg,
    pub fullscreen: FullscreenCfg,
}

/// 一个时间段。`from` / `to` 是 `HH:MM`，支持跨午夜（如 23:00–04:59）。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GreetingSlot {
    pub from: String,
    pub to: String,
    pub texts: Vec<String>,
    pub actions: Vec<String>,
}

impl GreetingSlot {
    fn matches(&self, minutes: u32) -> bool {
        let (Some(from), Some(to)) = (parse_hm(&self.from), parse_hm(&self.to)) else {
            return false;
        };
        if from <= to {
            minutes >= from && minutes <= to
        } else {
            minutes >= from || minutes <= to
        }
    }
}

pub fn parse_hm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if h > 23 || m > 59 {
        None
    } else {
        Some(h * 60 + m)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GreetingsConfig {
    /// 离开至少这么久才问好（屏幕点亮 / 解锁 / 恢复）。
    pub min_away_sec: u32,
    /// 两次问好的最小间隔。
    pub cooldown_sec: u32,
    /// 超过这个时长算“好久不见”。
    pub long_away_sec: u32,
    /// 启动后延迟多久问好，避开首帧加载。
    pub startup_delay_ms: u32,
    pub long_away_texts: Vec<String>,
    pub long_away_actions: Vec<String>,
    /// 当前时间不属于任何 slot 时的兜底。
    pub fallback_texts: Vec<String>,
    pub fallback_actions: Vec<String>,
    pub slots: Vec<GreetingSlot>,
}

impl Default for GreetingsConfig {
    fn default() -> Self {
        Self {
            min_away_sec: 60,
            cooldown_sec: 600,
            long_away_sec: 86_400,
            startup_delay_ms: 1_500,
            long_away_texts: Vec::new(),
            long_away_actions: Vec::new(),
            fallback_texts: Vec::new(),
            fallback_actions: Vec::new(),
            slots: Vec::new(),
        }
    }
}

impl GreetingsConfig {
    pub fn slot_for(&self, minutes: u32) -> Option<&GreetingSlot> {
        self.slots.iter().find(|s| s.matches(minutes))
    }
}

/// 数工资的报数时机与文案。放在 events.json 里，改完右键「重载素材」生效。
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SalaryConfig {
    /// 动作播到几成才报数，0.0~1.0。1.0 = 播完才说。
    pub say_at_ratio: f32,
    pub texts: SalaryTexts,
}

impl Default for SalaryConfig {
    fn default() -> Self {
        Self {
            say_at_ratio: 0.72,
            texts: SalaryTexts::default(),
        }
    }
}

impl SalaryConfig {
    /// 按时长算出该延迟多少毫秒。夹在 0 和整段时长之间，防止配置写飞。
    pub fn delay_ms(&self, total_ms: u32) -> u32 {
        let r = self.say_at_ratio.clamp(0.0, 1.0);
        (total_ms as f32 * r) as u32
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SalaryTexts {
    /// 还没到上班时间
    pub before: String,
    /// 上班中 / 下班后共用 `{amount}` 占位
    pub during: String,
    pub after: String,
    /// 今天是休息日（周末 / 法定假日）时说的话
    pub rest: String,
}

impl Default for SalaryTexts {
    fn default() -> Self {
        Self {
            before: "还没上班呢".to_string(),
            during: "今天赚了 ¥{amount}".to_string(),
            after: "下班啦，赚了 ¥{amount}".to_string(),
            rest: "今天休息，不打工".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EventsFile {
    /// 两次“会切动画”的事件之间的最小间隔。
    pub global_cooldown_sec: u32,
    pub sensors: SensorsConfig,
    pub bubble_style: BubbleStyle,
    pub events: HashMap<String, EventRule>,
    pub greetings: GreetingsConfig,
    pub salary: SalaryConfig,
}

impl Default for EventsFile {
    fn default() -> Self {
        Self {
            global_cooldown_sec: 8,
            sensors: SensorsConfig::default(),
            bubble_style: BubbleStyle::default(),
            events: HashMap::new(),
            greetings: GreetingsConfig::default(),
            salary: SalaryConfig::default(),
        }
    }
}

impl EventsFile {
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let stripped = strip_line_comments(text);
        serde_json::from_str(&stripped).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn rule(&self, key: EventKey) -> Option<&EventRule> {
        self.events.get(key.as_str())
    }
}

/// 气泡模板里可用的变量。
#[derive(Debug, Clone, Default)]
pub struct Vars {
    pub pct: Option<u8>,
    pub name: Option<String>,
    pub size: Option<u64>,
    pub time: Option<String>,
}

pub fn render(template: &str, vars: &Vars) -> String {
    let mut s = template.to_string();
    if let Some(v) = vars.pct {
        s = s.replace("{pct}", &v.to_string());
    }
    if let Some(v) = &vars.name {
        s = s.replace("{name}", v);
    }
    if let Some(v) = vars.size {
        s = s.replace("{size}", &format_size(v));
    }
    if let Some(v) = &vars.time {
        s = s.replace("{time}", v);
    }
    s
}

pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// 当前本地时间：(时, 分)。
pub fn local_time() -> (u32, u32) {
    unsafe {
        let mut st: windows_sys::Win32::Foundation::SYSTEMTIME = std::mem::zeroed();
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut st);
        (st.wHour as u32, st.wMinute as u32)
    }
}

pub fn local_hhmm() -> String {
    let (h, m) = local_time();
    format!("{h:02}:{m:02}")
}
