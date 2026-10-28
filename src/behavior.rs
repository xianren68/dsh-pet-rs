//! 行为配置：动画池、权重、物理参数。
//!
//! 直接对应原项目 `assets/config.jsonc` 的 `animations` / `animationWeights` /
//! `physics` 三段，去掉余额/碎碎念/工作状态这些需要宿主接口的档位。

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Behavior {
    /// 待机池（等概率抽）
    pub idle: Vec<String>,
    /// 转向池（播完翻转朝向）
    pub turn: Vec<String>,
    /// 拖拽中播放
    pub drag: Vec<String>,
    /// 点击回应池（随机抽 1 个）
    pub clicks: Vec<String>,
    pub moves: Moves,
    /// 随机动作分类池
    pub categories: Vec<Category>,
    pub weights: Weights,
    pub physics: Physics,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            idle: vec!["待机呼吸休闲".into()],
            turn: vec!["东张西望".into()],
            drag: vec!["被鼠标拖拽悬空反馈".into()],
            clicks: vec![],
            moves: Moves::default(),
            categories: vec![],
            weights: Weights::default(),
            physics: Physics::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Moves {
    pub default: MoveParams,
    pub actions: Vec<MoveAction>,
}

impl Default for Moves {
    fn default() -> Self {
        Self {
            default: MoveParams::default(),
            actions: vec![],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MoveParams {
    pub min_dist: u32,
    pub max_dist: u32,
    pub margin: u32,
    pub lead_sec: f32,
    pub tail_sec: f32,
}

impl Default for MoveParams {
    fn default() -> Self {
        Self {
            min_dist: 60,
            max_dist: 240,
            margin: 20,
            lead_sec: 2.0,
            tail_sec: 2.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveAction {
    pub name: String,
    #[serde(default)]
    pub min_dist: Option<u32>,
    #[serde(default)]
    pub max_dist: Option<u32>,
    #[serde(default)]
    pub margin: Option<u32>,
    #[serde(default)]
    pub lead_sec: Option<f32>,
    #[serde(default)]
    pub tail_sec: Option<f32>,
}

impl MoveAction {
    /// 合并 default 与自身覆盖项。
    pub fn params(&self, d: &MoveParams) -> MoveParams {
        MoveParams {
            min_dist: self.min_dist.unwrap_or(d.min_dist),
            max_dist: self.max_dist.unwrap_or(d.max_dist),
            margin: self.margin.unwrap_or(d.margin),
            lead_sec: self.lead_sec.unwrap_or(d.lead_sec),
            tail_sec: self.tail_sec.unwrap_or(d.tail_sec),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub weight: u32,
    /// 带文字、镜像会颠倒 → 朝右时不播
    pub no_mirror: bool,
    pub actions: Vec<String>,
}

impl Default for Category {
    fn default() -> Self {
        Self {
            id: String::new(),
            weight: 0,
            no_mirror: false,
            actions: vec![],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Weights {
    pub idle: u32,
    pub turn: u32,
    #[serde(rename = "move")]
    pub move_w: u32,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            idle: 10,
            turn: 5,
            move_w: 5,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Physics {
    pub gravity: f32,
    pub restitution: f32,
    pub ground_friction: f32,
    pub ceiling_bounce: bool,
    pub throw_power: f32,
    pub max_throw: f32,
    /// 宠物固定：true = 不自己走、不自己翻朝向
    pub fixed: bool,
}

impl Default for Physics {
    fn default() -> Self {
        Self {
            gravity: 1400.0,
            restitution: 0.78,
            ground_friction: 2.5,
            ceiling_bounce: true,
            throw_power: 1.0,
            max_throw: 2200.0,
            fixed: false,
        }
    }
}

impl Behavior {
    pub fn load(path: &std::path::Path) -> Result<Behavior, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        // 允许 // 行注释，方便和原项目配置对照着改
        let stripped = strip_line_comments(&text);
        serde_json::from_str(&stripped).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// 分类池权重合计。
    pub fn category_weight(&self) -> u32 {
        self.categories.iter().map(|c| c.weight).sum()
    }
}

/// 去掉 `//` 行注释（字符串内的不碰），让配置能带说明。
fn strip_line_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let bytes: Vec<char> = line.chars().collect();
        let mut in_str = false;
        let mut esc = false;
        let mut cut = bytes.len();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = !in_str;
            } else if !in_str && c == '/' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
                cut = i;
                break;
            }
            i += 1;
        }
        out.extend(&bytes[..cut]);
        out.push('\n');
    }
    out
}
