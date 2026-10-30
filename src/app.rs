//! 宠物状态机：动画链、拖拽、抛掷物理、朝向与位移、上屏。
//!
//! 行为规则来自 `assets/behaviors.json`（对应原项目 config.jsonc 的 animations /
//! animationWeights / physics 三段）：
//! 每段动画播完按权重抽下一档 —— 待机 / 转向 / 移动 / 随机分类，
//! 分类内先按分类权重抽类、再类内等概率抽动作。
//!
//! 环境事件（电量、网络、下载、空闲、锁屏、全屏、问好）来自
//! `assets/events.json`，代码里没有动作映射表。

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SetWindowPos, ShowWindow, HWND_TOPMOST, SW_HIDE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_SHOWNOACTIVATE,
};

use crate::behavior::{Behavior, Category, MoveAction};
use crate::bubble;
use crate::clip::{Clip, Stream};
use crate::event::{self, EventKey, EventsFile, Vars};
use crate::sense::{Sense, Signal};
use crate::surface::Surface;
use crate::util::{cursor, work_area, work_area_at, Rng};

/// `--debug-events` 的日志文件。优先配置目录，不可写时退到临时目录。
static EVENT_LOG: OnceLock<PathBuf> = OnceLock::new();
static EVENT_LOG_WARNED: OnceLock<()> = OnceLock::new();
static CONFIG_WARNED: OnceLock<()> = OnceLock::new();

/// 宠物朝向。
///
/// 素材**原生朝左**（原项目里 `facing === 'right'` 才叫「被镜像」）。
/// 所以 `Left` = 原样显示，`Right` = 整帧水平镜像。
/// 弄反会导致两个可见症状：走路倒着走、带文字的动画字是反的。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    Left,
    Right,
}

impl Facing {
    fn flip(self) -> Self {
        match self {
            Facing::Right => Facing::Left,
            Facing::Left => Facing::Right,
        }
    }
}

/// 当前这段动画属于哪一档——决定播完之后怎么接下一段。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Turn,
    Move,
    Category,
    Click,
    Drag,
    Manual,
    /// 事件动作；`event_hold` 决定播完是否定格重播。
    Event,
}

struct Drag {
    grab: (f32, f32),
    trail: Vec<(u32, f32, f32)>,
    start_ms: u32,
}

struct MovePlan {
    start_ms: u32,
    lead_ms: u32,
    dur_ms: u32,
    from_x: f32,
    dx: f32,
}

struct PendingEvent {
    key: EventKey,
    action: Option<String>,
    text: Option<String>,
    priority: u8,
    interrupt: bool,
    hold: bool,
    no_mirror: bool,
    at: u32,
}

#[derive(Clone, Copy, Debug)]
enum GreetReason {
    Startup,
    Display,
    Unlock,
    Resume,
    Simulated,
}

/// 设置面板用的开关集合。
#[derive(Clone, Copy)]
pub struct EventToggles {
    pub battery: bool,
    pub network: bool,
    pub download: bool,
    pub idle: bool,
    pub lock: bool,
    pub fullscreen: bool,
    pub greet: bool,
    pub greet_startup: bool,
    pub bubble: bool,
    pub bubble_names: bool,
}

#[derive(Clone, Copy)]
pub enum Toggle {
    Battery,
    Network,
    Download,
    Idle,
    Lock,
    Fullscreen,
    Greet,
    GreetStartup,
    Bubble,
    BubbleNames,
}

pub struct App {
    hwnd: HWND,
    surface: Surface,

    anims: HashMap<String, PathBuf>,
    loaded: HashMap<String, Clip>,

    beh: Behavior,
    events: EventsFile,
    sense: Sense,
    phase: Phase,
    cur_name: String,
    frame: usize,
    frame_ms: u32,
    px: Vec<u32>,
    out: Vec<u32>,
    stream: Option<Stream>,

    pos: (f32, f32),
    vel: (f32, f32),
    flying: bool,
    facing: Facing,
    drag: Option<Drag>,
    move_plan: Option<MovePlan>,

    pending: Vec<PendingEvent>,
    last_fire: HashMap<EventKey, u32>,
    last_event_at: u32,
    event_hold: bool,
    last_greet: u32,
    startup_greet_at: Option<u32>,
    /// 数工资的报数时刻：播到动作的 `sayAtRatio` 处才弹气泡，
    /// 免得数钱还没数完就先把结果念出来了。
    salary_at: Option<u32>,
    /// 配置目录（节假日缓存也放这儿）
    cfg_dir: std::path::PathBuf,
    /// 当年的节假日表。拿不到就是 None，日薪回落到 21.75。
    holidays: Option<crate::holiday::Holidays>,
    /// 下次重试读缓存的时间点
    holiday_retry_at: u32,
    /// 调试开关：启动时读一次 `DSH_DUMP_PET`，避免每帧都查环境变量
    dump_pet: bool,
    last_wake_at: Option<u32>,
    fullscreen_hidden: bool,
    bubble_until: u32,

    last_tick: u32,
    rng: Rng,
    visible: bool,
    dirty: bool,
    topmost_tick: u32,
    debug: bool,
    test_minutes: Option<u32>,

    /// 素材目录（只读）
    assets: PathBuf,
    /// 配置目录（可写，通常 %APPDATA%\dsh-pet-rs）
    cfg: PathBuf,
    /// 面板写下的用户偏好（尺寸/不透明度/固定/权重/事件开关/位置）
    user: UserSettings,
}

impl App {
    pub fn new(hwnd: HWND, assets: PathBuf, cfg: PathBuf) -> Result<Self, String> {
        let beh = Behavior::load(&assets.join("behaviors.json")).unwrap_or_else(|e| {
            eprintln!("[behavior] {e}，改用内置默认");
            Behavior::default()
        });
        let events = EventsFile::load(&assets.join("events.json")).unwrap_or_else(|e| {
            eprintln!("[events] {e}，环境事件已关闭");
            EventsFile::default()
        });
        let anims = scan_anims(&assets)?;
        if anims.is_empty() {
            return Err(format!(
                "{} 里没有可用的素材（.webp/.apng/.gif）",
                assets.display()
            ));
        }

        // 只探测待机那一条，其余按需加载——106 段全探一遍要半分钟
        let first = beh
            .idle
            .first()
            .cloned()
            .filter(|n| anims.contains_key(n))
            .or_else(|| anims.keys().min().cloned())
            .ok_or("没有可用素材")?;
        let clip = Clip::probe(anims.get(&first).unwrap(), &first)?;
        // 显示尺寸优先用上次保存的宽度，缺省就是素材原尺寸
        let user = UserSettings::load(&cfg);
        let want_w = user.width.clamp(200, 900);
        let want_h = ((want_w as f32 * 9.0 / 16.0).round() as u32).max(1);
        let surface = Surface::new(want_w as i32, want_h as i32).ok_or("创建 DIB 表面失败")?;

        let wa: RECT = work_area();
        let margin = 48;
        let fallback = (
            (wa.right - want_w as i32 - margin) as f32,
            (wa.bottom - want_h as i32 - margin) as f32,
        );
        let pos = match (user.x, user.y) {
            (Some(x), Some(y)) => {
                let here = work_area_at(
                    x + want_w as i32 / 2,
                    y + want_h as i32 / 2,
                );
                if x + want_w as i32 > here.left
                    && x < here.right
                    && y + want_h as i32 > here.top
                    && y < here.bottom
                {
                    (x as f32, y as f32)
                } else {
                    fallback
                }
            }
            _ => fallback,
        };
        // 记住的位置只要还和某块屏幕有交集就沿用，但保证整只宠物可见。
        let here = work_area_at(
            pos.0 as i32 + want_w as i32 / 2,
            pos.1 as i32 + want_h as i32 / 2,
        );
        let pos = (
            pos.0.clamp(
                here.left as f32,
                (here.right - want_w as i32).max(here.left) as f32,
            ),
            pos.1.clamp(
                here.top as f32,
                (here.bottom - want_h as i32).max(here.top) as f32,
            ),
        );

        let now = unsafe { GetTickCount() };
        let has_greeting = !events.greetings.slots.is_empty()
            || !events.greetings.long_away_texts.is_empty()
            || !events.greetings.long_away_actions.is_empty()
            || !events.greetings.fallback_texts.is_empty()
            || !events.greetings.fallback_actions.is_empty();
        let startup_greet_at = if user.greet_startup && has_greeting {
            Some(now.wrapping_add(events.greetings.startup_delay_ms))
        } else {
            None
        };
        let sense = Sense::new(&events.sensors);

        let mut loaded = HashMap::new();
        loaded.insert(first.clone(), clip);

        let mut app = Self {
            hwnd,
            surface,
            anims,
            loaded,
            beh,
            events,
            sense,
            phase: Phase::Idle,
            cur_name: first,
            frame: 0,
            frame_ms: 0,
            px: Vec::new(),
            out: Vec::new(),
            stream: None,
            pos,
            vel: (0.0, 0.0),
            flying: false,
            facing: Facing::Left,
            drag: None,
            move_plan: None,
            pending: Vec::new(),
            last_fire: HashMap::new(),
            last_event_at: 0,
            event_hold: false,
            last_greet: 0,
            startup_greet_at,
            salary_at: None,
            cfg_dir: cfg.clone(),
            holidays: {
                let st = crate::holiday::today();
                crate::holiday::load(&cfg, st.wYear as i32)
            },
            holiday_retry_at: now.wrapping_add(8_000),
            dump_pet: std::env::var_os("DSH_DUMP_PET").is_some(),
            last_wake_at: None,
            fullscreen_hidden: false,
            bubble_until: 0,
            last_tick: now,
            rng: Rng::new(),
            visible: true,
            dirty: true,
            topmost_tick: 0,
            debug: false,
            test_minutes: None,
            assets,
            cfg,
            user,
        };
        app.apply_user();
        app.validate_events();

        // 没有节假日缓存就后台拉一次（一年一条，拉到就再不用联网）
        if app.holidays.is_none() {
            let year = crate::holiday::today().wYear as i32;
            crate::holiday::fetch_async(app.cfg.clone(), year);
        }

        let name = app.cur_name.clone();
        app.open(&name);
        app.paint();
        eprintln!(
            "[init] anims={} events={} size={}x{} pos=({:.0},{:.0})",
            app.anims.len(),
            app.events.events.len(),
            app.surface.w,
            app.surface.h,
            app.pos.0,
            app.pos.1
        );
        Ok(app)
    }

    #[allow(dead_code)]
    pub fn current_name(&self) -> &str {
        &self.cur_name
    }

    /// 全部素材清单，给后台预热缓存用。
    pub fn anim_list(&self) -> Vec<(String, std::path::PathBuf)> {
        self.anims
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub fn categories(&self) -> Vec<(String, Vec<String>)> {
        let mut out = vec![
            ("待机".to_string(), self.beh.idle.clone()),
            ("转向".to_string(), self.beh.turn.clone()),
            (
                "移动".to_string(),
                self.beh.moves.actions.iter().map(|a| a.name.clone()).collect(),
            ),
            ("点击".to_string(), self.beh.clicks.clone()),
            ("拖拽".to_string(), self.beh.drag.clone()),
        ];
        for c in &self.beh.categories {
            out.push((c.id.clone(), c.actions.clone()));
        }
        out
    }

    pub fn toggle_visible(&mut self) -> bool {
        self.visible = !self.visible;
        if self.visible {
            if !self.fullscreen_hidden {
                unsafe {
                    ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                }
                self.dirty = true;
                self.paint();
            }
        } else {
            unsafe {
                ShowWindow(self.hwnd, SW_HIDE);
            }
            bubble::hide();
            self.bubble_until = 0;
        }
        self.visible
    }

    /// 菜单手动点播。
    pub fn play_name(&mut self, name: &str) {
        if self.anims.contains_key(name) {
            // 带文字的片段不能镜像：朝右时先翻回朝左再播，
            // 否则整帧一翻，画面里的字就是反的。
            if self.facing == Facing::Right && self.beh.is_no_mirror_clip(name) {
                self.facing = Facing::Left;
            }
            self.event_hold = false;
            self.open(name);
            self.phase = Phase::Manual;
            self.dirty = true;
            self.paint();
        }
    }

    /// 面板初始化时读一次工资配置：开关 + 四个时间 + 月薪。
    pub fn salary_cfg(&self) -> (bool, String, String, String, String, u32) {
        (
            self.user.salary,
            self.user.work_start.clone(),
            self.user.work_end.clone(),
            self.user.lunch_start.clone(),
            self.user.lunch_end.clone(),
            self.user.salary_month,
        )
    }

    /// 数工资开关是否打开（右键菜单据此决定要不要显示那条菜单项）。
    pub fn salary_on(&self) -> bool {
        self.user.salary
    }

    /// 数工资：播一次「数工资」动作。报数由 `open` 里的钩子负责 ——
    /// 这样不管是右键点的还是随机抽到的，都会报一次，不用两处维护。
    pub fn count_salary(&mut self) {
        if self.user.salary && self.anims.contains_key("数工资") {
            self.play_name("数工资");
        }
    }

    /// 当月实际工作日数。有节假日表就按真实日历算，否则回落到 21.75。
    fn salary_workdays(&self) -> f32 {
        if let Some(h) = &self.holidays {
            let st = crate::holiday::today();
            let n = h.workdays_in_month(st.wYear as i32, st.wMonth as u32, &st);
            if n > 0 {
                return n as f32;
            }
        }
        crate::holiday::FALLBACK_WORKDAYS
    }

    /// 把今天赚到的数字报在气泡里。开关没开、配置不合法都静默跳过。
    fn report_salary(&mut self, now: u32) {
        if !self.user.salary {
            return;
        }
        // 周末 / 法定假日点它就别算钱了 —— 人没上班，算出来是假的
        if let Some(h) = &self.holidays {
            let st = crate::holiday::today();
            if !h.is_workday(&st) {
                let text = self.events.salary.texts.rest.clone();
                self.show_bubble(&text, now);
                self.debug_log("salary: rest day");
                return;
            }
        }
        let workdays = self.salary_workdays();
        let Some(p) = crate::salary::progress(&self.user, workdays) else {
            eprintln!("[salary] 上下班 / 午休时间不合法，跳过");
            return;
        };
        let text = crate::salary::bubble_text(&p, &self.events.salary.texts);
        self.show_bubble(&text, now);
        self.debug_log(&format!("salary {text}"));
    }

    // ---- 播放 ----

    fn ensure(&mut self, name: &str) -> Result<(), String> {
        if self.loaded.contains_key(name) {
            return Ok(());
        }
        let path = self
            .anims
            .get(name)
            .cloned()
            .ok_or_else(|| format!("没有素材 {name}"))?;
        let c = Clip::probe(&path, name)?;
        self.loaded.insert(name.to_string(), c);
        Ok(())
    }

    fn open(&mut self, name: &str) {
        let path = match self.anims.get(name) {
            Some(p) => p.clone(),
            None => {
                eprintln!("[clip] 找不到 {name}");
                return;
            }
        };
        if let Err(e) = self.ensure(name) {
            eprintln!("[clip] {e}");
            return;
        }
        match Stream::open(&path) {
            Ok(mut s) => match s.next_frame() {
                Some(px) => {
                    self.px = px;
                    self.stream = Some(s);
                    self.cur_name = name.to_string();
                    self.frame = 0;
                    self.frame_ms = 0;
                    self.move_plan = None;
                    // 「数工资」是一段会说话的动作：不论从哪条路径播到它都要报数。
                    // 但不立刻说 —— 挂一个时刻，等动作推进到 sayAtRatio 再弹，
                    // 否则钱还没数完就先把结果念出来了。
                    if name == "数工资" {
                        let total = self.loaded.get(name).map(|c| c.total_ms()).unwrap_or(0);
                        let delay = self.events.salary.delay_ms(total);
                        let now = unsafe { GetTickCount() };
                        self.salary_at = Some(now.wrapping_add(delay));
                    }
                }
                None => eprintln!("[clip] {name} 解不出第一帧"),
            },
            Err(e) => eprintln!("[clip] {e}"),
        }
    }

    /// 动画链：按权重抽下一档。
    fn advance_chain(&mut self) {
        if self.phase == Phase::Turn {
            self.facing = self.facing.flip();
        }

        let fixed = self.beh.physics.fixed;
        let w_idle = self.beh.weights.idle;
        let w_turn = if fixed { 0 } else { self.beh.weights.turn };
        let w_move = if fixed { 0 } else { self.beh.weights.move_w };
        let w_cat = self.beh.category_weight();
        let total = w_idle + w_turn + w_move + w_cat;
        if total == 0 {
            self.go_idle();
            return;
        }

        let r = self.rng.next_u32() % total;
        if r < w_idle {
            self.go_idle();
        } else if r < w_idle + w_turn {
            let pool = self.beh.turn.clone();
            match pick(&mut self.rng, &pool) {
                Some(name) => {
                    self.open(&name);
                    self.phase = Phase::Turn;
                }
                None => self.go_idle(),
            }
        } else if r < w_idle + w_turn + w_move {
            self.start_move();
        } else {
            self.start_category();
        }
    }

    fn go_idle(&mut self) {
        let pool = self.beh.idle.clone();
        match pick(&mut self.rng, &pool) {
            Some(n) => {
                self.open(&n);
                self.phase = Phase::Idle;
            }
            None => self.phase = Phase::Idle,
        }
    }

    fn start_category(&mut self) {
        let cats = self.beh.categories.clone();
        if cats.is_empty() {
            self.go_idle();
            return;
        }
        // 朝右时跳过 noMirror 的分类（带文字，镜像会颠倒）
        let usable: Vec<&Category> = cats
            .iter()
            .filter(|c| !(c.no_mirror && self.facing == Facing::Right))
            .collect();
        if usable.is_empty() {
            self.go_idle();
            return;
        }
        let weights: Vec<u32> = usable.iter().map(|c| c.weight).collect();
        let total: u32 = weights.iter().sum();
        if total == 0 {
            self.go_idle();
            return;
        }
        let mut r = self.rng.next_u32() % total;
        let mut chosen = usable.len() - 1;
        for (i, w) in weights.iter().enumerate() {
            if r < *w {
                chosen = i;
                break;
            }
            r -= *w;
        }
        let actions = usable[chosen].actions.clone();
        match pick(&mut self.rng, &actions) {
            Some(n) => {
                self.open(&n);
                self.phase = Phase::Category;
            }
            None => self.go_idle(),
        }
    }

    fn start_move(&mut self) {
        let acts = self.beh.moves.actions.clone();
        let defaults = self.beh.moves.default.clone();
        if acts.is_empty() {
            self.go_idle();
            return;
        }
        let idx = (self.rng.next_u32() as usize) % acts.len();
        let act: &MoveAction = &acts[idx];
        let p = act.params(&defaults);

        let span = p.max_dist.saturating_sub(p.min_dist) + 1;
        let dist = p.min_dist + (self.rng.next_u32() % span);
        let wa = self.monitor_area();
        let w = self.surface.w as f32;
        let lo = wa.left as f32 + p.margin as f32;
        let hi = (wa.right as f32 - w - p.margin as f32).max(lo);

        // 先按当前朝向走；那侧空间不够（贴边了）就转身朝另一侧
        let mut dir = if self.facing == Facing::Right { 1.0 } else { -1.0 };
        let room = if dir > 0.0 { hi - self.pos.0 } else { self.pos.0 - lo };
        if room < 40.0 {
            let other = if dir > 0.0 { self.pos.0 - lo } else { hi - self.pos.0 };
            if other > 40.0 {
                dir = -dir;
                self.facing = self.facing.flip();
            }
        }

        let target = (self.pos.0 + dist as f32 * dir).clamp(lo, hi);
        let dx = target - self.pos.0;

        let name = act.name.clone();
        self.open(&name);
        self.phase = Phase::Move;

        // 动画首尾各有定格，位移只发生在中间那段
        let total = self
            .loaded
            .get(&name)
            .map(|c| c.total_ms())
            .unwrap_or(2000);
        let lead = (p.lead_sec * 1000.0) as u32;
        let tail = (p.tail_sec * 1000.0) as u32;
        let dur = total.saturating_sub(lead + tail).max(1);
        self.move_plan = Some(MovePlan {
            start_ms: unsafe { GetTickCount() },
            lead_ms: lead,
            dur_ms: dur,
            from_x: self.pos.0,
            dx,
        });
    }

    fn react(&mut self) {
        let pool = self.beh.clicks.clone();
        if let Some(n) = pick(&mut self.rng, &pool) {
            self.event_hold = false;
            self.open(&n);
            self.phase = Phase::Click;
        }
    }

    // ---- 主循环 ----

    pub fn tick(&mut self) {
        let now = unsafe { GetTickCount() };
        let mut dt = now.wrapping_sub(self.last_tick) as f32 / 1000.0;
        self.last_tick = now;
        dt = dt.clamp(0.0, 0.06);

        let mut changed = false;
        if self.flying {
            self.step_physics(dt);
            changed = true;
        }
        if self.step_move(now) {
            changed = true;
        }
        if self.step_anim((dt * 1000.0) as u32) {
            changed = true;
        }

        if self.bubble_until != 0 && now >= self.bubble_until {
            bubble::hide();
            self.bubble_until = 0;
        }

        if let Some(at) = self.startup_greet_at {
            if now >= at {
                self.startup_greet_at = None;
                self.maybe_greet(GreetReason::Startup, None, now, false);
                changed = true;
            }
        }

        // 数工资：等到动作推进到设定比例，再报数
        if let Some(at) = self.salary_at {
            if now >= at {
                self.salary_at = None;
                self.report_salary(now);
                changed = true;
            }
        }

        // 节假日缓存是后台线程拉的，这里定期看看文件到位没有
        if self.holidays.is_none() && now >= self.holiday_retry_at {
            let st = crate::holiday::today();
            match crate::holiday::load(&self.cfg_dir, st.wYear as i32) {
                Some(h) => {
                    self.holidays = Some(h);
                    self.debug_log("holidays loaded");
                }
                None => self.holiday_retry_at = now.wrapping_add(60_000),
            }
        }

        self.try_events(false);

        if self.dirty || changed {
            self.paint();
        }

        self.topmost_tick = self.topmost_tick.wrapping_add(1);
        if self.topmost_tick >= 1800 {
            self.topmost_tick = 0;
            self.reassert_topmost();
        }
    }

    fn step_move(&mut self, now: u32) -> bool {
        let Some(mp) = &self.move_plan else {
            return false;
        };
        let el = now.wrapping_sub(mp.start_ms);
        let mut changed = false;
        if el > mp.lead_ms {
            let t = ((el - mp.lead_ms) as f32 / mp.dur_ms as f32).min(1.0);
            self.pos.0 = mp.from_x + mp.dx * t;
            changed = true;
        }
        if el > mp.lead_ms + mp.dur_ms {
            self.move_plan = None;
        }
        changed
    }

    fn paint(&mut self) {
        if self.px.is_empty() || !self.visible || self.fullscreen_hidden {
            return;
        }
        let dw = self.surface.w as usize;
        let dh = self.surface.h as usize;
        let (sw, sh) = self
            .loaded
            .get(&self.cur_name)
            .map(|c| (c.w as usize, c.h as usize))
            .unwrap_or((dw, dh));

        // 兜底：先把表面抹成全透明。正常情况下 upload 会整块覆盖，
        // 但万一某帧解出来的像素数少于画布（解码异常），剩下的像素
        // 会保留上一帧内容，表现为画面上零散的方块残留。
        self.surface.pixels().fill(0);
        if self.px.len() < dw * dh {
            eprintln!(
                "[paint] 帧不完整 {} < {} ({}x{})",
                self.px.len(),
                dw * dh,
                dw,
                dh
            );
        }

        if sw == dw && sh == dh {
            // 原尺寸直传，零开销（默认就是这条路径）
            match self.facing {
                Facing::Left => self.surface.upload(&self.px),
                Facing::Right => self.surface.upload_flipped(&self.px),
            }
        } else {
            self.out = crate::scale::scale(&self.px, sw, sh, dw, dh);
            match self.facing {
                Facing::Left => self.surface.upload(&self.out),
                Facing::Right => self.surface.upload_flipped(&self.out),
            }
        }
        self.surface.present(
            self.hwnd,
            self.pos.0 as i32,
            self.pos.1 as i32,
            self.user.alpha,
        );
        // 调试：每隔 20 帧把实际渲染结果存一张，用来判断花屏是数据问题还是窗口问题
        if self.dump_pet && self.frame % 20 == 0 {
            let (w, h) = (dw as u32, dh as u32);
            let px = self.surface.pixels();
            let mut img = image::RgbaImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let p = px[(y * w + x) as usize];
                    img.put_pixel(
                        x,
                        y,
                        image::Rgba([(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]),
                    );
                }
            }
            let path = std::env::temp_dir().join(format!("pet_dump_{:03}.png", self.frame));
            let _ = img.save(&path);
        }
        self.dirty = false;
        bubble::follow(
            self.pos.0 as i32,
            self.pos.1 as i32,
            self.surface.w,
            self.surface.h,
        );
    }

    pub fn width(&self) -> u32 {
        self.surface.w.max(1) as u32
    }

    /// 改显示宽度：重建 DIB，夹回当前屏幕内，写盘。
    pub fn set_width(&mut self, w: u32) {
        let w = w.clamp(200, 900);
        if w == self.surface.w as u32 {
            return;
        }
        let h = ((w as f32 * 9.0 / 16.0).round() as u32).max(1);
        let Some(s) = Surface::new(w as i32, h as i32) else {
            return;
        };
        self.surface = s;

        let wa = self.monitor_area();
        self.pos.0 = self
            .pos
            .0
            .min((wa.right - w as i32 - 8) as f32)
            .max(wa.left as f32);
        self.pos.1 = self
            .pos
            .1
            .min((wa.bottom - h as i32 - 8) as f32)
            .max(wa.top as f32);

        self.user.width = w;
        self.save_position();
        self.dirty = true;
        self.paint();
    }

    pub fn alpha(&self) -> u8 {
        self.user.alpha
    }

    /// 整窗不透明度：0-255，太低会看不见，面板里限到 40 以上
    pub fn set_alpha(&mut self, a: u8) {
        self.user.alpha = a.clamp(40, 255);
        self.save_user();
        self.dirty = true;
        self.paint();
    }

    pub fn fixed(&self) -> bool {
        self.beh.physics.fixed
    }

    /// 面板改工资配置。时间串原样存下，合法性交给 `salary::progress` 判断，
    /// 面板这边不拦 —— 输到一半的 `09:` 也要能存进内存，等改完再算。
    pub fn set_salary(
        &mut self,
        on: bool,
        work_start: String,
        work_end: String,
        lunch_start: String,
        lunch_end: String,
        month: u32,
    ) {
        self.user.salary = on;
        self.user.work_start = work_start;
        self.user.work_end = work_end;
        self.user.lunch_start = lunch_start;
        self.user.lunch_end = lunch_end;
        self.user.salary_month = month;
        self.save_user();
    }

    pub fn set_fixed(&mut self, on: bool) {
        self.user.fixed = on;
        self.beh.physics.fixed = on;
        if on {
            self.move_plan = None;
        }
        self.save_user();
    }

    /// 三档权重：0=待机 1=转向 2=走动
    pub fn weights(&self) -> (u32, u32, u32) {
        (
            self.beh.weights.idle,
            self.beh.weights.turn,
            self.beh.weights.move_w,
        )
    }

    pub fn set_weight(&mut self, which: usize, v: u32) {
        let v = v.min(100);
        match which {
            0 => {
                self.user.w_idle = Some(v);
                self.beh.weights.idle = v;
            }
            1 => {
                self.user.w_turn = Some(v);
                self.beh.weights.turn = v;
            }
            _ => {
                self.user.w_move = Some(v);
                self.beh.weights.move_w = v;
            }
        }
        self.save_user();
    }

    fn apply_user(&mut self) {
        if let Some(v) = self.user.w_idle {
            self.beh.weights.idle = v;
        }
        if let Some(v) = self.user.w_turn {
            self.beh.weights.turn = v;
        }
        if let Some(v) = self.user.w_move {
            self.beh.weights.move_w = v;
        }
        self.beh.physics.fixed = self.user.fixed;
    }

    fn save_user(&self) {
        if let Err(e) = self.user.save(&self.cfg) {
            if CONFIG_WARNED.set(()).is_ok() {
                eprintln!(
                    "[config] 保存设置失败 {}: {e}",
                    self.cfg.join("settings.json").display()
                );
            }
        }
    }

    fn save_position(&mut self) {
        self.user.x = Some(self.pos.0.round() as i32);
        self.user.y = Some(self.pos.1.round() as i32);
        self.save_user();
    }

    pub fn save_on_exit(&mut self) {
        self.save_position();
    }

    fn step_anim(&mut self, dt_ms: u32) -> bool {
        self.frame_ms += dt_ms;
        let mut changed = false;
        let mut guard = 0;
        loop {
            let (delay, len) = match self.loaded.get(&self.cur_name) {
                Some(c) => {
                    if self.frame >= c.delays.len() {
                        (0, c.delays.len())
                    } else {
                        (c.delays[self.frame], c.delays.len())
                    }
                }
                None => return changed,
            };
            if self.frame >= len {
                self.on_clip_end();
                return true;
            }
            if self.frame_ms < delay {
                break;
            }
            self.frame_ms -= delay;
            guard += 1;
            if guard > 3 {
                self.frame_ms = 0;
                break;
            }
            self.frame += 1;
            changed = true;
            if self.frame >= len {
                self.on_clip_end();
                return true;
            }
            let next = self.stream.as_mut().and_then(|s| s.next_frame());
            match next {
                Some(px) => self.px = px,
                None => {
                    self.on_clip_end();
                    return true;
                }
            }
        }
        changed
    }

    fn on_clip_end(&mut self) {
        // 拖拽动画循环播
        if self.phase == Phase::Drag && self.drag.is_some() {
            let name = self.cur_name.clone();
            self.open(&name);
            return;
        }
        // 睡眠等 hold 事件：播完从头循环，直到收到退出事件
        if self.phase == Phase::Event && self.event_hold {
            let name = self.cur_name.clone();
            self.open(&name);
            return;
        }
        if !self.try_events(true) {
            self.advance_chain();
        }
    }

    fn step_physics(&mut self, dt: f32) {
        let ph = self.beh.physics.clone();
        let (w, h) = (self.surface.w as f32, self.surface.h as f32);

        self.vel.1 += ph.gravity * dt;
        self.pos.0 += self.vel.0 * dt;
        self.pos.1 += self.vel.1 * dt;

        let wa = self.monitor_area();
        let (left, top) = (wa.left as f32, wa.top as f32);
        let (right, bottom) = (wa.right as f32, wa.bottom as f32);

        if self.pos.0 < left {
            self.pos.0 = left;
            self.vel.0 = -self.vel.0 * ph.restitution;
        } else if self.pos.0 + w > right {
            self.pos.0 = right - w;
            self.vel.0 = -self.vel.0 * ph.restitution;
        }

        if self.pos.1 < top {
            self.pos.1 = top;
            if ph.ceiling_bounce {
                self.vel.1 = -self.vel.1 * ph.restitution;
            } else {
                self.vel.1 = 0.0;
            }
        }

        let ground = bottom - h;
        let mut on_ground = false;
        if self.pos.1 >= ground {
            self.pos.1 = ground;
            if self.vel.1 > 60.0 {
                self.vel.1 = -self.vel.1 * ph.restitution;
            } else {
                self.vel.1 = 0.0;
                on_ground = true;
            }
        }

        if on_ground {
            // groundFriction 是衰减率：v *= exp(-k·dt)
            self.vel.0 *= (-ph.ground_friction * dt).exp();
        }

        let speed = (self.vel.0 * self.vel.0 + self.vel.1 * self.vel.1).sqrt();
        if on_ground && speed < 30.0 {
            self.vel = (0.0, 0.0);
            self.flying = false;
            self.save_position();
            self.advance_chain();
        }
    }

    // ---- 鼠标 ----

    pub fn begin_drag(&mut self) {
        let (cx, cy) = cursor();
        self.drag = Some(Drag {
            grab: (cx as f32 - self.pos.0, cy as f32 - self.pos.1),
            trail: Vec::new(),
            start_ms: unsafe { GetTickCount() },
        });
        self.flying = false;
        self.vel = (0.0, 0.0);
        self.move_plan = None;
        self.event_hold = false;
        let pool = self.beh.drag.clone();
        if let Some(n) = pick(&mut self.rng, &pool) {
            self.open(&n);
            self.phase = Phase::Drag;
        }
        self.dirty = true;
    }

    pub fn drag_move(&mut self) {
        let now = unsafe { GetTickCount() };
        let (cx, cy) = cursor();
        let Some(d) = self.drag.as_mut() else {
            return;
        };
        self.pos = (cx as f32 - d.grab.0, cy as f32 - d.grab.1);
        d.trail.push((now, self.pos.0, self.pos.1));
        while let Some(&(t, _, _)) = d.trail.first() {
            if now.wrapping_sub(t) > 120 {
                d.trail.remove(0);
            } else {
                break;
            }
        }
        self.dirty = true;
    }

    pub fn end_drag(&mut self) {
        let Some(d) = self.drag.take() else {
            return;
        };
        self.save_position();

        // 位移小、时间短 = 单击，播个回应动作
        let held = unsafe { GetTickCount() }.wrapping_sub(d.start_ms);
        let moved = d
            .trail
            .first()
            .map(|&(_, x0, y0)| {
                let (x1, y1) = self.pos;
                ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt()
            })
            .unwrap_or(0.0);
        if held < 260 && moved < 6.0 {
            self.react();
            return;
        }

        let mut vx = 0.0;
        let mut vy = 0.0;
        if d.trail.len() >= 2 {
            let (t0, x0, y0) = d.trail[0];
            let (t1, x1, y1) = *d.trail.last().unwrap();
            let dt = t1.wrapping_sub(t0) as f32 / 1000.0;
            if dt > 0.008 {
                vx = (x1 - x0) / dt;
                vy = (y1 - y0) / dt;
            }
        }

        let power = self.beh.physics.throw_power.max(0.1);
        let max = self.beh.physics.max_throw.max(200.0);
        vx *= power;
        vy *= power;
        let speed = (vx * vx + vy * vy).sqrt();
        if speed > max {
            let k = max / speed;
            vx *= k;
            vy *= k;
        }

        if speed > 120.0 {
            self.vel = (vx, vy);
            self.flying = true;
            self.phase = Phase::Manual;
        } else {
            self.advance_chain();
        }
        self.dirty = true;
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn home(&mut self) {
        let wa = self.monitor_area();
        let (w, h) = (self.surface.w, self.surface.h);
        self.pos = ((wa.right - w - 48) as f32, (wa.bottom - h - 48) as f32);
        self.vel = (0.0, 0.0);
        self.flying = false;
        self.move_plan = None;
        self.save_position();
        self.dirty = true;
        self.paint();
    }

    pub fn reload(&mut self) -> Result<(), String> {
        let beh = Behavior::load(&self.assets.join("behaviors.json"))?;
        let events = EventsFile::load(&self.assets.join("events.json")).unwrap_or_else(|e| {
            eprintln!("[events] {e}，环境事件已关闭");
            EventsFile::default()
        });
        self.beh = beh;
        self.events = events;
        self.sense.update_config(&self.events.sensors);
        self.anims = scan_anims(&self.assets)?;
        self.loaded.clear();
        self.pending.clear();
        self.event_hold = false;
        self.apply_user();
        self.validate_events();
        let name = self.cur_name.clone();
        if self.anims.contains_key(&name) {
            self.open(&name);
        } else {
            self.go_idle();
        }
        self.dirty = true;
        self.paint();
        Ok(())
    }

    // ---- 环境事件 ----

    pub fn set_debug(&mut self, on: bool) {
        self.debug = on;
    }

    pub fn skip_startup_greeting(&mut self) {
        self.startup_greet_at = None;
    }

    pub fn set_test_minutes(&mut self, minutes: Option<u32>) {
        self.test_minutes = minutes;
    }

    pub fn toggles(&self) -> EventToggles {
        EventToggles {
            battery: self.user.evt_battery,
            network: self.user.evt_network,
            download: self.user.evt_download,
            idle: self.user.evt_idle,
            lock: self.user.evt_lock,
            fullscreen: self.user.evt_fullscreen,
            greet: self.user.greet_screen,
            greet_startup: self.user.greet_startup,
            bubble: self.user.bubble,
            bubble_names: self.user.bubble_names,
        }
    }

    pub fn set_toggle(&mut self, which: Toggle, on: bool) {
        match which {
            Toggle::Battery => self.user.evt_battery = on,
            Toggle::Network => self.user.evt_network = on,
            Toggle::Download => self.user.evt_download = on,
            Toggle::Idle => self.user.evt_idle = on,
            Toggle::Lock => self.user.evt_lock = on,
            Toggle::Fullscreen => self.user.evt_fullscreen = on,
            Toggle::Greet => self.user.greet_screen = on,
            Toggle::GreetStartup => self.user.greet_startup = on,
            Toggle::Bubble => self.user.bubble = on,
            Toggle::BubbleNames => self.user.bubble_names = on,
        }
        if !self.user.bubble {
            bubble::hide();
            self.bubble_until = 0;
        }
        if !self.user.evt_fullscreen && self.fullscreen_hidden {
            self.fullscreen_hidden = false;
            if self.visible {
                unsafe {
                    ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                }
                self.dirty = true;
                self.paint();
            }
        } else if self.user.evt_fullscreen && self.sense.is_fullscreen() {
            self.set_fullscreen(true);
        }
        self.save_user();
    }

    pub fn poll_senses(&mut self) {
        let now = unsafe { GetTickCount() };
        let signals = self.sense.poll(now);
        self.handle_signals(signals, now);
        self.try_events(false);
    }

    pub fn poll_power(&mut self) {
        let now = unsafe { GetTickCount() };
        let signals = self.sense.poll_battery(now);
        self.handle_signals(signals, now);
        self.try_events(false);
    }

    pub fn on_display(&mut self, on: bool) {
        let now = unsafe { GetTickCount() };
        if !on {
            let _ = self.sense.set_display(false, now);
            self.pending.clear();
            bubble::hide();
            self.bubble_until = 0;
            self.debug_log("display off");
            return;
        }
        let away = self.sense.set_display(true, now);
        self.debug_log(&format!("display on away={away:?}"));
        // 熄屏期间传感器可能攒下了 idleSleep / 旧的下载事件，亮屏时先丢掉。
        self.pending.clear();
        if self.sense.is_locked() {
            return;
        }
        self.wake_from_hold();
        self.maybe_greet(GreetReason::Display, away, now, false);
        self.try_events(false);
    }

    pub fn on_suspend(&mut self) {
        let now = unsafe { GetTickCount() };
        self.sense.on_suspend(now);
        self.pending.clear();
        bubble::hide();
        self.bubble_until = 0;
        self.debug_log("suspend");
    }

    pub fn on_resume(&mut self) {
        let now = unsafe { GetTickCount() };
        let away = self.sense.on_resume(now);
        if self.wake_recent(now) {
            return;
        }
        self.last_wake_at = Some(now);
        self.pending.clear();
        self.wake_from_hold();
        // 问好和“恢复”动作可以同时发生：问好只负责气泡，动作仍按 events.json 播。
        self.maybe_greet(GreetReason::Resume, away, now, false);
        self.enqueue(EventKey::Resume, self.vars(now), now);
        self.try_events(false);
    }

    pub fn on_session(&mut self, locked: bool) {
        let now = unsafe { GetTickCount() };
        if locked {
            let _ = self.sense.set_locked(true, now);
            // 锁屏期间不保留解锁/下载等旧事件，否则解锁后可能突然补播。
            self.pending.clear();
            self.event_hold = false;
            self.phase = Phase::Idle;
            bubble::hide();
            self.bubble_until = 0;
            self.enqueue(EventKey::SessionLock, self.vars(now), now);
            self.try_events(true);
            self.debug_log("session lock");
            return;
        }
        let away = self.sense.set_locked(false, now);
        if self.wake_recent(now) {
            return;
        }
        self.last_wake_at = Some(now);
        self.pending.clear();
        self.wake_from_hold();
        self.maybe_greet(GreetReason::Unlock, away, now, false);
        self.enqueue(EventKey::SessionUnlock, self.vars(now), now);
        self.debug_log("session unlock");
        self.try_events(false);
    }

    /// 多屏/分辨率变化后，把宠物夹回当前显示器。
    pub fn on_display_change(&mut self) {
        self.clamp_to_monitor();
        self.dirty = true;
        self.paint();
    }

    fn wake_recent(&self, now: u32) -> bool {
        self.last_wake_at
            .map(|t| now.wrapping_sub(t) < 3_000)
            .unwrap_or(false)
    }

    fn wake_from_hold(&mut self) {
        if self.event_hold {
            self.event_hold = false;
            if self.phase == Phase::Event {
                self.phase = Phase::Idle;
            }
            self.dirty = true;
        }
    }

    fn vars(&self, now: u32) -> Vars {
        let _ = now;
        Vars {
            time: Some(event::local_hhmm()),
            ..Default::default()
        }
    }

    fn handle_signals(&mut self, signals: Vec<Signal>, now: u32) {
        for sig in signals {
            match sig {
                Signal::BatteryLow { pct } => {
                    self.debug_log(&format!("battery low {pct}%"));
                    self.enqueue(
                        EventKey::BatteryLow,
                        Vars {
                            pct: Some(pct),
                            ..self.vars(now)
                        },
                        now,
                    );
                }
                Signal::BatteryCritical { pct } => {
                    self.debug_log(&format!("battery critical {pct}%"));
                    self.enqueue(
                        EventKey::BatteryCritical,
                        Vars {
                            pct: Some(pct),
                            ..self.vars(now)
                        },
                        now,
                    );
                }
                Signal::BatteryCharging => {
                    self.debug_log("battery charging");
                    self.enqueue(EventKey::BatteryCharging, self.vars(now), now);
                }
                Signal::BatteryFull => {
                    self.debug_log("battery full");
                    self.enqueue(EventKey::BatteryFull, self.vars(now), now);
                }
                Signal::NetworkDown => {
                    self.debug_log("network down");
                    self.enqueue(EventKey::NetworkDown, self.vars(now), now);
                }
                Signal::NetworkUp => {
                    self.debug_log("network up");
                    self.enqueue(EventKey::NetworkUp, self.vars(now), now);
                }
                Signal::DownloadStart { name } => {
                    self.debug_log(&format!("download start {name}"));
                    self.enqueue(
                        EventKey::DownloadStart,
                        Vars {
                            name: Some(self.display_name(name)),
                            ..self.vars(now)
                        },
                        now,
                    );
                }
                Signal::DownloadDone { name, size } => {
                    self.debug_log(&format!("download done {name} {size}"));
                    self.enqueue(
                        EventKey::DownloadDone,
                        Vars {
                            name: Some(self.display_name(name)),
                            size: Some(size),
                            ..self.vars(now)
                        },
                        now,
                    );
                }
                Signal::DownloadFailed { name } => {
                    self.debug_log(&format!("download failed {name}"));
                    self.enqueue(
                        EventKey::DownloadFailed,
                        Vars {
                            name: Some(self.display_name(name)),
                            ..self.vars(now)
                        },
                        now,
                    );
                }
                Signal::IdleSleep => {
                    self.debug_log("idle sleep");
                    self.enqueue(EventKey::IdleSleep, self.vars(now), now);
                }
                Signal::IdleWake => {
                    self.debug_log("idle wake");
                    self.enqueue(EventKey::IdleWake, self.vars(now), now);
                    if self.phase == Phase::Event && self.event_hold {
                        self.event_hold = false;
                        self.phase = Phase::Idle;
                    }
                    self.try_events(true);
                }
                Signal::FullscreenEnter => {
                    self.debug_log("fullscreen enter");
                    self.set_fullscreen(true);
                }
                Signal::FullscreenExit => {
                    self.debug_log("fullscreen exit");
                    self.set_fullscreen(false);
                }
            }
        }
    }

    fn display_name(&self, name: String) -> String {
        if self.user.bubble_names {
            name
        } else {
            "文件".to_string()
        }
    }

    fn enqueue(&mut self, key: EventKey, vars: Vars, now: u32) {
        if !self.group_enabled(key) {
            return;
        }
        let Some(rule) = self.events.rule(key).cloned() else {
            return;
        };
        if !rule.enabled() {
            return;
        }
        if self.pending.iter().any(|p| p.key == key) {
            return;
        }
        if let Some(last) = self.last_fire.get(&key) {
            if now.wrapping_sub(*last) < rule.cooldown_sec() * 1000 {
                return;
            }
        }

        let text = if self.user.bubble {
            rule.bubble
                .as_ref()
                .map(|t| event::render(t, &vars))
                .filter(|s| !s.trim().is_empty())
        } else {
            None
        };
        let action = self.pick_action(&rule.actions);
        if text.is_none() && action.is_none() {
            return;
        }

        self.debug_log(&format!("queue {}", key.as_str()));
        self.pending.push(PendingEvent {
            key,
            action,
            text,
            priority: rule.priority(),
            interrupt: rule.interrupt(),
            hold: rule.hold(),
            no_mirror: rule.no_mirror(),
            at: now,
        });
        self.pending.sort_by_key(|p| std::cmp::Reverse(p.priority));
    }

    /// 只启动动作事件时返回 true；纯气泡事件处理完继续找下一个。
    fn try_events(&mut self, allow_cut: bool) -> bool {
        if self.fullscreen_hidden {
            return false;
        }
        if !matches!(
            self.phase,
            Phase::Idle | Phase::Turn | Phase::Move | Phase::Category
        ) {
            return false;
        }
        let now = unsafe { GetTickCount() };
        self.pending
            .retain(|p| now.wrapping_sub(p.at) <= 15_000);

        loop {
            let idx = self
                .pending
                .iter()
                .enumerate()
                .filter(|(_, p)| p.action.is_none() || allow_cut || p.interrupt)
                .filter(|(_, p)| {
                    p.action.is_none()
                        || now.wrapping_sub(self.last_event_at)
                            >= self.events.global_cooldown_sec * 1000
                })
                .max_by_key(|(_, p)| p.priority)
                .map(|(i, _)| i);
            let Some(idx) = idx else {
                return false;
            };
            let p = self.pending.remove(idx);
            self.last_fire.insert(p.key, now);

            if let Some(text) = p.text.clone() {
                self.show_bubble(&text, now);
            }

            if let Some(action) = p.action {
                if p.no_mirror && self.facing == Facing::Right {
                    self.facing = Facing::Left;
                }
                self.open(&action);
                self.phase = Phase::Event;
                self.event_hold = p.hold;
                self.last_event_at = now;
                self.dirty = true;
                self.debug_log(&format!("start {} -> {}", p.key.as_str(), action));
                return true;
            }
            self.debug_log(&format!("bubble {}", p.key.as_str()));
        }
    }

    fn show_bubble(&mut self, text: &str, now: u32) {
        if !self.user.bubble || !self.visible || self.fullscreen_hidden {
            return;
        }
        bubble::show(
            text,
            self.pos.0 as i32,
            self.pos.1 as i32,
            self.surface.w,
            self.surface.h,
            &self.events.bubble_style,
        );
        self.bubble_until = now.wrapping_add(self.events.bubble_style.duration_ms());
    }

    fn maybe_greet(
        &mut self,
        reason: GreetReason,
        away: Option<u32>,
        now: u32,
        force: bool,
    ) -> bool {
        if !force {
            if reason_is_startup(reason) {
                if !self.user.greet_startup {
                    return false;
                }
            } else if !self.user.greet_screen {
                return false;
            }
        }
        if !self.sense.display_on() || self.sense.is_locked() {
            return false;
        }
        if self.fullscreen_hidden || !self.visible {
            return false;
        }

        let away = away.unwrap_or(0);
        if !force {
            if !matches!(reason, GreetReason::Startup | GreetReason::Simulated)
                && away < self.events.greetings.min_away_sec
            {
                return false;
            }
            if !matches!(reason, GreetReason::Startup | GreetReason::Simulated)
                && now.wrapping_sub(self.last_greet)
                    < self.events.greetings.cooldown_sec * 1000
            {
                return false;
            }
        }

        let minutes = self.test_minutes.unwrap_or_else(|| {
            let (h, m) = event::local_time();
            h * 60 + m
        });
        let g = &self.events.greetings;
        let (texts, actions) = if away >= g.long_away_sec
            && (!g.long_away_texts.is_empty() || !g.long_away_actions.is_empty())
        {
            (g.long_away_texts.clone(), g.long_away_actions.clone())
        } else if let Some(slot) = g.slot_for(minutes) {
            (slot.texts.clone(), slot.actions.clone())
        } else {
            (g.fallback_texts.clone(), g.fallback_actions.clone())
        };

        let text = if self.user.bubble {
            pick(&mut self.rng, &texts)
                .map(|t| event::render(&t, &self.vars(now)))
                .filter(|s| !s.trim().is_empty())
        } else {
            None
        };
        let action = self.pick_action(&actions);
        if text.is_none() && action.is_none() {
            return false;
        }

        self.last_greet = now;
        self.debug_log(&format!("greet {reason:?} away={away}s"));
        self.pending.push(PendingEvent {
            key: EventKey::Greeting,
            action,
            text,
            priority: 20,
            interrupt: true,
            hold: false,
            no_mirror: false,
            at: now,
        });
        self.try_events(true);
        true
    }

    fn pick_action(&mut self, actions: &[String]) -> Option<String> {
        if actions.is_empty() {
            return None;
        }
        let n = actions.len();
        for _ in 0..n {
            let name = &actions[(self.rng.next_u32() as usize) % n];
            if self.anims.contains_key(name) {
                return Some(name.clone());
            }
        }
        None
    }

    fn group_enabled(&self, key: EventKey) -> bool {
        match key {
            EventKey::BatteryLow
            | EventKey::BatteryCritical
            | EventKey::BatteryCharging
            | EventKey::BatteryFull => self.user.evt_battery,
            EventKey::NetworkDown | EventKey::NetworkUp => self.user.evt_network,
            EventKey::DownloadStart | EventKey::DownloadDone | EventKey::DownloadFailed => {
                self.user.evt_download
            }
            EventKey::IdleSleep | EventKey::IdleWake => self.user.evt_idle,
            EventKey::SessionLock | EventKey::SessionUnlock | EventKey::Resume => {
                self.user.evt_lock
            }
            EventKey::Greeting => true,
        }
    }

    fn set_fullscreen(&mut self, on: bool) {
        if !self.user.evt_fullscreen {
            return;
        }
        if on == self.fullscreen_hidden {
            return;
        }
        self.fullscreen_hidden = on;
        if on {
            if self.visible {
                unsafe {
                    ShowWindow(self.hwnd, SW_HIDE);
                }
            }
            bubble::hide();
            self.bubble_until = 0;
            self.debug_log("fullscreen hide");
        } else {
            if self.visible {
                unsafe {
                    ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                }
                self.dirty = true;
                self.reassert_topmost();
                self.paint();
            }
            self.try_events(false);
            self.debug_log("fullscreen restore");
        }
    }

    fn clamp_to_monitor(&mut self) {
        let wa = self.monitor_area();
        let (w, h) = (self.surface.w, self.surface.h);
        self.pos.0 = self
            .pos
            .0
            .clamp(wa.left as f32, (wa.right - w).max(wa.left) as f32);
        self.pos.1 = self
            .pos
            .1
            .clamp(wa.top as f32, (wa.bottom - h).max(wa.top) as f32);
        self.save_position();
    }

    fn monitor_area(&self) -> RECT {
        work_area_at(
            self.pos.0 as i32 + self.surface.w / 2,
            self.pos.1 as i32 + self.surface.h / 2,
        )
    }

    fn reassert_topmost(&self) {
        if !self.visible || self.fullscreen_hidden {
            return;
        }
        unsafe {
            SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    fn validate_events(&self) {
        for (key, rule) in &self.events.events {
            self.validate_actions(&format!("events.{key}"), &rule.actions);
        }
        let g = &self.events.greetings;
        self.validate_actions("greetings.longAway", &g.long_away_actions);
        self.validate_actions("greetings.fallback", &g.fallback_actions);
        for (i, slot) in g.slots.iter().enumerate() {
            let name = format!("greetings.slots[{i}]");
            self.validate_actions(&name, &slot.actions);
            if event::parse_hm(&slot.from).is_none() || event::parse_hm(&slot.to).is_none() {
                eprintln!("[events] {name}: from/to 不是合法的 HH:MM，已跳过");
            }
        }
    }

    fn validate_actions(&self, context: &str, actions: &[String]) {
        for action in actions {
            if !self.anims.contains_key(action) {
                eprintln!("[events] {context}: 找不到动作「{action}」");
            }
        }
    }

    fn debug_log(&self, msg: &str) {
        if !self.debug {
            return;
        }
        eprintln!("[event] {msg}");
        let path = EVENT_LOG.get_or_init(|| {
            let primary = self.cfg.join("events.log");
            if std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(true)
                .open(&primary)
                .is_ok()
            {
                primary
            } else {
                // 某些受限环境不允许程序往 %APPDATA% 新建文件；调试日志退到 TEMP。
                let fallback = std::env::temp_dir().join("dsh-pet-rs-events.log");
                eprintln!("[event] 日志回退到 {}", fallback.display());
                fallback
            }
        });
        match std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(path)
        {
            Ok(mut f) => {
                let _ = writeln!(f, "{msg}");
            }
            Err(e) => {
                if EVENT_LOG_WARNED.set(()).is_ok() {
                    eprintln!("[event] 写日志失败 {}: {e}", path.display());
                }
            }
        }
    }

    // ---- 调试模拟 ----

    pub fn simulate(&mut self, spec: &str) {
        let now = unsafe { GetTickCount() };
        for part in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let mut it = part.split(':');
            let kind = it.next().unwrap_or("");
            let a = it.next();
            let b = it.next();
            match kind {
                "batteryLow" => self.handle_signals(
                    vec![Signal::BatteryLow {
                        pct: a.and_then(|s| s.parse().ok()).unwrap_or(15),
                    }],
                    now,
                ),
                "batteryCritical" => self.handle_signals(
                    vec![Signal::BatteryCritical {
                        pct: a.and_then(|s| s.parse().ok()).unwrap_or(8),
                    }],
                    now,
                ),
                "batteryCharging" => self.handle_signals(vec![Signal::BatteryCharging], now),
                "batteryFull" => self.handle_signals(vec![Signal::BatteryFull], now),
                "networkDown" => self.handle_signals(vec![Signal::NetworkDown], now),
                "networkUp" => self.handle_signals(vec![Signal::NetworkUp], now),
                "downloadStart" => self.handle_signals(
                    vec![Signal::DownloadStart {
                        name: a.unwrap_or("file.zip").to_string(),
                    }],
                    now,
                ),
                "downloadDone" => self.handle_signals(
                    vec![Signal::DownloadDone {
                        name: a.unwrap_or("file.zip").to_string(),
                        size: b.and_then(|s| s.parse().ok()).unwrap_or(1_048_576),
                    }],
                    now,
                ),
                "downloadFailed" => self.handle_signals(
                    vec![Signal::DownloadFailed {
                        name: a.unwrap_or("file.zip").to_string(),
                    }],
                    now,
                ),
                "idleSleep" => self.handle_signals(vec![Signal::IdleSleep], now),
                "idleWake" => self.handle_signals(vec![Signal::IdleWake], now),
                "sessionLock" => self.on_session(true),
                "sessionUnlock" => {
                    self.last_wake_at = None;
                    self.on_session(false);
                }
                "resume" => {
                    self.last_wake_at = None;
                    self.on_resume();
                }
                "fullscreenEnter" => self.set_fullscreen(true),
                "fullscreenExit" => self.set_fullscreen(false),
                "greet" => {
                    // `greet:20:30` 里的时间被 split(':') 拆成了 a/b 两段
                    let hm = match (a, b) {
                        (Some(h), Some(m)) => format!("{h}:{m}"),
                        (Some(h), None) => h.to_string(),
                        _ => String::new(),
                    };
                    if let Some(minutes) = event::parse_hm(&hm) {
                        self.test_minutes = Some(minutes);
                    }
                    self.maybe_greet(GreetReason::Simulated, None, now, true);
                }
                _ => eprintln!("[simulate] 未知事件 {kind}"),
            }
        }
        self.try_events(true);
    }
}

fn pick(rng: &mut Rng, pool: &[String]) -> Option<String> {
    if pool.is_empty() {
        None
    } else {
        Some(pool[(rng.next_u32() as usize) % pool.len()].clone())
    }
}

fn reason_is_startup(reason: GreetReason) -> bool {
    matches!(reason, GreetReason::Startup | GreetReason::Simulated)
}

/// 扫素材目录：`assets/anims/*` 与 `assets/*`，同名以 anims 优先。
fn scan_anims(dir: &std::path::Path) -> Result<HashMap<String, PathBuf>, String> {
    let mut out = HashMap::new();
    let anims_dir = dir.join("anims");
    let dirs = if anims_dir.is_dir() {
        vec![dir.to_path_buf(), anims_dir]
    } else {
        vec![dir.to_path_buf()]
    };
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if matches!(ext.as_str(), "webp" | "apng" | "gif") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.insert(stem.to_string(), p);
                }
            }
        }
    }
    Ok(out)
}

fn settings_path(dir: &std::path::Path) -> PathBuf {
    dir.join("settings.json")
}

/// 面板能改的所有东西，一次性存 `%APPDATA%\dsh-pet-rs\settings.json`。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct UserSettings {
    #[serde(default = "d_width")]
    pub width: u32,
    #[serde(default = "d_alpha")]
    pub alpha: u8,
    #[serde(default)]
    pub fixed: bool,
    #[serde(default)]
    pub w_idle: Option<u32>,
    #[serde(default)]
    pub w_turn: Option<u32>,
    #[serde(default)]
    pub w_move: Option<u32>,
    #[serde(default = "d_true")]
    pub evt_battery: bool,
    #[serde(default = "d_true")]
    pub evt_network: bool,
    #[serde(default = "d_true")]
    pub evt_download: bool,
    #[serde(default = "d_true")]
    pub evt_idle: bool,
    #[serde(default = "d_true")]
    pub evt_lock: bool,
    #[serde(default = "d_true")]
    pub evt_fullscreen: bool,
    #[serde(default = "d_true")]
    pub greet_screen: bool,
    #[serde(default = "d_true")]
    pub greet_startup: bool,
    #[serde(default = "d_true")]
    pub bubble: bool,
    #[serde(default = "d_true")]
    pub bubble_names: bool,
    // ---- 数工资 ----
    #[serde(default)]
    pub salary: bool,
    #[serde(default = "d_work_start")]
    pub work_start: String,
    #[serde(default = "d_work_end")]
    pub work_end: String,
    #[serde(default = "d_lunch_start")]
    pub lunch_start: String,
    #[serde(default = "d_lunch_end")]
    pub lunch_end: String,
    #[serde(default = "d_salary_month")]
    pub salary_month: u32,
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
}

fn d_width() -> u32 {
    640
}
fn d_alpha() -> u8 {
    255
}
pub(crate) fn d_true() -> bool {
    true
}
fn d_work_start() -> String {
    "09:00".to_string()
}
fn d_work_end() -> String {
    "18:00".to_string()
}
fn d_lunch_start() -> String {
    "12:00".to_string()
}
fn d_lunch_end() -> String {
    "13:00".to_string()
}
fn d_salary_month() -> u32 {
    8000
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            width: d_width(),
            alpha: d_alpha(),
            fixed: false,
            w_idle: None,
            w_turn: None,
            w_move: None,
            evt_battery: true,
            evt_network: true,
            evt_download: true,
            evt_idle: true,
            evt_lock: true,
            evt_fullscreen: true,
            greet_screen: true,
            greet_startup: true,
            bubble: true,
            bubble_names: true,
            salary: false,
            work_start: d_work_start(),
            work_end: d_work_end(),
            lunch_start: d_lunch_start(),
            lunch_end: d_lunch_end(),
            salary_month: d_salary_month(),
            x: None,
            y: None,
        }
    }
}

impl UserSettings {
    pub fn load(dir: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(settings_path(dir)) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        let body = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(settings_path(dir), body + "\n")
    }
}
