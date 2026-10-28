//! 宠物状态机：动画链、拖拽、抛掷物理、朝向与位移、上屏。
//!
//! 行为规则来自 `assets/behaviors.json`（对应原项目 config.jsonc 的 animations /
//! animationWeights / physics 三段）：
//! 每段动画播完按权重抽下一档 —— 待机 / 转向 / 移动 / 随机分类，
//! 分类内先按分类权重抽类、再类内等概率抽动作。

use std::collections::HashMap;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::System::SystemInformation::GetTickCount;

use crate::behavior::{Behavior, Category, MoveAction};
use crate::clip::{Clip, Stream};
use crate::surface::Surface;
use crate::util::{cursor, work_area, Rng};

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

pub struct App {
    hwnd: windows_sys::Win32::Foundation::HWND,
    surface: Surface,

    anims: HashMap<String, PathBuf>,
    loaded: HashMap<String, Clip>,

    beh: Behavior,
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

    last_tick: u32,
    rng: Rng,
    visible: bool,
    /// 素材目录（只读）
    assets: PathBuf,
    /// 配置目录（可写，通常 %APPDATA%\dsh-pet-rs）
    cfg: PathBuf,
    /// 面板写下的用户偏好（尺寸/不透明度/固定/权重）
    user: UserSettings,
}

impl App {
    pub fn new(
        hwnd: windows_sys::Win32::Foundation::HWND,
        assets: PathBuf,
        cfg: PathBuf,
    ) -> Result<Self, String> {
        let beh = Behavior::load(&assets.join("behaviors.json")).unwrap_or_else(|e| {
            eprintln!("[behavior] {e}，改用内置默认");
            Behavior::default()
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
        let pos = (
            (wa.right - want_w as i32 - margin) as f32,
            (wa.bottom - want_h as i32 - margin) as f32,
        );

        let mut loaded = HashMap::new();
        loaded.insert(first.clone(), clip);

        let mut app = Self {
            hwnd,
            surface,
            anims,
            loaded,
            beh,
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
            last_tick: unsafe { GetTickCount() },
            rng: Rng::new(),
            visible: true,
            assets,
            cfg,
            user,
        };
        app.apply_user();

        let name = app.cur_name.clone();
        app.open(&name);
        app.paint();
        eprintln!(
            "[init] anims={} size={}x{} pos=({:.0},{:.0})",
            app.anims.len(),
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
        self.visible
    }

    pub fn repaint(&mut self) {
        self.paint();
    }

    /// 菜单手动点播。
    pub fn play_name(&mut self, name: &str) {
        if self.anims.contains_key(name) {
            // 带文字的片段不能镜像：朝右时先翻回朝左再播，
            // 否则整帧一翻，画面里的字就是反的。
            if self.facing == Facing::Right && self.beh.is_no_mirror_clip(name) {
                self.facing = Facing::Left;
            }
            self.open(name);
            self.phase = Phase::Manual;
            self.paint();
        }
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
        let wa: RECT = work_area();
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

        if self.flying {
            self.step_physics(dt);
        }
        self.step_move(now);
        self.step_anim((dt * 1000.0) as u32);
        self.paint();
    }

    fn step_move(&mut self, now: u32) {
        let Some(mp) = &self.move_plan else { return };
        let el = now.wrapping_sub(mp.start_ms);
        if el > mp.lead_ms {
            let t = ((el - mp.lead_ms) as f32 / mp.dur_ms as f32).min(1.0);
            self.pos.0 = mp.from_x + mp.dx * t;
        }
        if el > mp.lead_ms + mp.dur_ms {
            self.move_plan = None;
        }
    }

    fn paint(&mut self) {
        if self.px.is_empty() {
            return;
        }
        let dw = self.surface.w as usize;
        let dh = self.surface.h as usize;
        let (sw, sh) = self
            .loaded
            .get(&self.cur_name)
            .map(|c| (c.w as usize, c.h as usize))
            .unwrap_or((dw, dh));

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
    }

    pub fn width(&self) -> u32 {
        self.surface.w.max(1) as u32
    }

    /// 改显示宽度：重建 DIB，夹回屏幕内，写盘。
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

        let wa: RECT = work_area();
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
        self.save_user();
        self.paint();
    }

    pub fn alpha(&self) -> u8 {
        self.user.alpha
    }

    /// 整窗不透明度：0-255，太低会看不见，面板里限到 40 以上
    pub fn set_alpha(&mut self, a: u8) {
        self.user.alpha = a.clamp(40, 255);
        self.save_user();
        self.paint();
    }

    pub fn fixed(&self) -> bool {
        self.beh.physics.fixed
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
        let _ = self.user.save(&self.cfg);
    }

    fn step_anim(&mut self, dt_ms: u32) {
        self.frame_ms += dt_ms;
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
                None => return,
            };
            if self.frame >= len {
                self.on_clip_end();
                return;
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
            if self.frame >= len {
                self.on_clip_end();
                return;
            }
            let next = self.stream.as_mut().and_then(|s| s.next_frame());
            match next {
                Some(px) => self.px = px,
                None => {
                    self.on_clip_end();
                    return;
                }
            }
        }
    }

    fn on_clip_end(&mut self) {
        // 拖拽动画循环播
        if self.phase == Phase::Drag && self.drag.is_some() {
            let name = self.cur_name.clone();
            self.open(&name);
            return;
        }
        self.advance_chain();
    }

    fn step_physics(&mut self, dt: f32) {
        let ph = self.beh.physics.clone();
        let (w, h) = (self.surface.w as f32, self.surface.h as f32);

        self.vel.1 += ph.gravity * dt;
        self.pos.0 += self.vel.0 * dt;
        self.pos.1 += self.vel.1 * dt;

        let wa: RECT = work_area();
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
        let pool = self.beh.drag.clone();
        if let Some(n) = pick(&mut self.rng, &pool) {
            self.open(&n);
            self.phase = Phase::Drag;
        }
    }

    pub fn drag_move(&mut self) {
        let now = unsafe { GetTickCount() };
        let (cx, cy) = cursor();
        let Some(d) = self.drag.as_mut() else { return };
        self.pos = (cx as f32 - d.grab.0, cy as f32 - d.grab.1);
        d.trail.push((now, self.pos.0, self.pos.1));
        while let Some(&(t, _, _)) = d.trail.first() {
            if now.wrapping_sub(t) > 120 {
                d.trail.remove(0);
            } else {
                break;
            }
        }
    }

    pub fn end_drag(&mut self) {
        let Some(d) = self.drag.take() else { return };

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
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn home(&mut self) {
        let wa: RECT = work_area();
        let (w, h) = (self.surface.w, self.surface.h);
        self.pos = ((wa.right - w - 48) as f32, (wa.bottom - h - 48) as f32);
        self.vel = (0.0, 0.0);
        self.flying = false;
        self.move_plan = None;
        self.paint();
    }

    pub fn reload(&mut self) -> Result<(), String> {
        self.beh = Behavior::load(&self.assets.join("behaviors.json"))?;
        self.anims = scan_anims(&self.assets)?;
        self.loaded.clear();
        let name = self.cur_name.clone();
        if self.anims.contains_key(&name) {
            self.open(&name);
        } else {
            self.go_idle();
        }
        self.paint();
        Ok(())
    }
}

fn pick(rng: &mut Rng, pool: &[String]) -> Option<String> {
    if pool.is_empty() {
        None
    } else {
        Some(pool[(rng.next_u32() as usize) % pool.len()].clone())
    }
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
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
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

/// 面板能改的所有东西，一次性存 `assets/settings.json`。
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
}

fn d_width() -> u32 {
    640
}
fn d_alpha() -> u8 {
    255
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
