/*!
Wallpaper Engine 壁纸支持：包解析、落盘存储、贴图体检与本地取流。

第一阶段只做「装得进来、取得到字节」两件事：

* [`pkg`] 负责 `PKGM0019` / `PKGV0024` 两种容器的解析与解包（条目不压缩）；
* [`store`] 负责 `$APPDATA/<identifier>/wallpapers/` 这套布局、内容哈希 id、
  临时目录 + 原子 rename 的写入方式；
* [`tex_audit`] 只读 `.tex` 文件头，给出「这批贴图需要 GPU 的哪项能力」，
  **不转码**（转码是二期的事，见架构文档 §11.4）；
* [`protocol`] 是 `hayenai-we` 取流协议的纯逻辑（路径解析、MIME、缓存头、Range），
  与 Tauri 的适配层留在 `apps/app/src/we_protocol.rs`；
* [`steam`] 只读地扫本机 Steam 库里的 WE 工坊壁纸（`workshop/content/431960`），
  并把「最近一次扫描结果」当作导入的白名单 —— 不碰 Steam 账号、不发网络请求。

模块内的错误一律是中文单句：调用方可以原样显示给用户，不会有堆栈、
也不会有绝对路径（原始 `io::Error` 只作为 `source` 留给日志）。
*/

pub mod pkg;
pub mod protocol;
pub mod steam;
pub mod store;
pub mod tex_audit;

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 所有壁纸数据都放在 `$APPDATA/<identifier>/wallpapers/` 下。
pub const WALLPAPERS_DIR_NAME: &str = "wallpapers";

/// 壁纸目录的根：`$APPDATA/<identifier>/wallpapers`。
pub fn wallpapers_root(app_data_dir: &std::path::Path) -> PathBuf {
    app_data_dir.join(WALLPAPERS_DIR_NAME)
}

/// 壁纸相关的错误。`Display` 即用户可见文案（中文单句，无路径、无堆栈）。
#[derive(Debug, thiserror::Error)]
pub enum WallpaperError {
    /// magic 不是 `PKGM0019` / `PKGV0024`。
    #[error("不是 Wallpaper Engine 壁纸包")]
    NotAPackage,

    /// 文件短于最小容器头（`magicLen + magic + count`）。
    #[error("壁纸包不完整，文件太短")]
    PackageTooShort,

    /// 索引或数据区声明的长度超出文件实际长度，通常是下载不完整。
    #[error("壁纸包已损坏，可能没有下载完整")]
    PackageTruncated,

    /// 条目表本身大得不像话（或已损坏），无法在一次读取内解析。
    #[error("壁纸包的文件目录异常，无法解析")]
    PackageTableTooLarge,

    /// 条目名长度超过 4096 字节。
    #[error("壁纸包里的文件名过长")]
    EntryNameTooLong,

    /// 条目名不是合法 UTF-8。
    #[error("壁纸包里的文件名不是合法文本")]
    EntryNameInvalid,

    /// 条目名含 `..`、绝对路径、盘符、NUL、超长等（zip-slip 一类）。
    #[error("壁纸包里含有不安全的路径")]
    UnsafePath,

    /// 解包总量超过 2 GB。
    #[error("壁纸包解出来超过 2 GB，已中止")]
    PackageTooLarge,

    /// 既没有 `project.json`，也无法由场景子包合成。
    #[error("壁纸包里没有 project.json，无法识别")]
    MissingProject,

    /// `project.json` 存在但不是合法 JSON。
    #[error("壁纸包的 project.json 不是合法 JSON")]
    BadProject(#[from] serde_json::Error),

    /// `project.json` 大得不像话（超过 1 MiB），不像能用的清单。
    #[error("壁纸包的 project.json 过大")]
    ProjectTooLarge,

    /// 用户给的路径不存在。
    #[error("找不到这个文件或目录")]
    SourceMissing,

    /// 包 id 不满足 [`store::is_valid_id`]。
    #[error("壁纸包 id 不合法")]
    InvalidId,

    /// 请求的包不在 `packages/` 下。
    #[error("找不到这个壁纸包")]
    UnknownPackage,

    /// 文件读写失败。**不**在文案里带上裸 `io::Error`：它含绝对路径。
    #[error("读写壁纸文件失败")]
    Io(#[from] std::io::Error),

    /// `config.json` 读写失败。
    #[error("壁纸配置读写失败")]
    Config,
}

impl WallpaperError {
    /// 用户可见文案。与 `Display` 相同，单独留一个名字是为了调用方读起来明确。
    pub fn user_message(&self) -> String {
        self.to_string()
    }
}

/// 本模块内部统一用这个别名，避免和 `theseus::Result` 混用。
pub type Result<T> = std::result::Result<T, WallpaperError>;

/// 壁纸类型，与 `project.json` 的 `type` 字段一一对应（未识别的落到 `Other`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperType {
    Scene,
    Video,
    Web,
    Application,
    Other,
}

impl WallpaperType {
    /// 把 `project.json.type` 归一化。大小写不敏感，未知值归到 `Other`。
    pub fn from_project_type(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "scene" => Self::Scene,
            "video" => Self::Video,
            "web" => Self::Web,
            "application" => Self::Application,
            _ => Self::Other,
        }
    }

    /// 第一期能不能画出来。`web` / `application` 一律「暂不支持」。
    pub fn is_supported(self) -> bool {
        matches!(self, Self::Scene | Self::Video)
    }

    /// 归一化之后的小写名字，和 `project.json.type` 的规范写法一致。
    ///
    /// 给「只想要一个字符串」的调用方用（[`steam::SteamWallpaper::kind`]）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scene => "scene",
            Self::Video => "video",
            Self::Web => "web",
            Self::Application => "application",
            Self::Other => "other",
        }
    }

    /// 界面上给用户看的名字。
    pub fn label(self) -> &'static str {
        match self {
            Self::Scene => "场景",
            Self::Video => "视频",
            Self::Web => "网页",
            Self::Application => "应用",
            Self::Other => "其他",
        }
    }
}

/// `.tex` 体检结果：只回答「这个包需要什么」，不回答「这台机器画不画得出」。
///
/// 判定依据是 `webwallgl`（2.1.0，见架构文档 §11.3）里真实存在的分支：
/// ETC2（format 3/5）与 BPTC（12）只走 GPU 一条路，且取常量前不查扩展，
/// 所以这两类贴图在 ANGLE-D3D11（WebView2 的形态）上会被静默拒收。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TexAudit {
    /// 命中 format 3（ETC2_RGB8）或 5（ETC2_RGBA8）。
    pub etc2: bool,
    /// 命中 format 4/6/7（DXT1/DXT3/DXT5，S3TC）。
    pub dxt: bool,
    /// 命中 format 12（BPTC / BC7）。
    pub bptc: bool,
    /// 扫到的 `.tex` 总数。
    pub total: usize,
    /// 「可能画不出来但又不是 ETC2/BPTC」的张数：文件头读不出来的（旧容器、
    /// 损坏），以及渲染器分支里根本不认的格式值。
    pub unreadable: usize,
    /// 格式名 → 张数，用来在界面上举例。
    pub formats: BTreeMap<String, usize>,
}

impl TexAudit {
    /// 命中「GPU 独有」格式时给一句中文提示，供导入结果里的 `warnings` 用。
    pub fn warning(&self) -> Option<String> {
        if self.etc2 {
            Some(
                "包里有 ETC2 贴图，当前设备可能渲染不出背景".to_string(),
            )
        } else if self.bptc {
            Some("包里有 BPTC 贴图，当前设备可能渲染不出背景".to_string())
        } else if self.unreadable > 0 {
            Some("包里有读不出格式的贴图，可能显示异常".to_string())
        } else {
            None
        }
    }
}

/// 一个已安装壁纸的元数据（`packages/<id>/.hayenai-meta.json` 的内容）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperMeta {
    /// 目录内容 SHA-1 的前 12 位，带 `ws-` / `local-` 前缀。
    pub id: String,
    /// `project.json.type` 归一化后的类型。
    #[serde(rename = "type")]
    pub kind: WallpaperType,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    /// 工坊 id（本地导入的包为空串）。
    pub workshop_id: String,
    /// 入口文件（场景是 `scene.json`，视频是那个 mp4 的相对路径）。
    pub file: String,
    /// 预览图的相对路径（包内不存在则为 `None`）。
    pub preview: Option<String>,
    /// 包内文件数。
    pub entries: usize,
    /// 包内文件总字节数。
    pub bytes: u64,
    /// 安装时间，Unix 毫秒。
    pub installed_at: i64,
    /// `project.json.general.properties` 是否存在。
    pub has_properties: bool,
    /// 贴图体检结果。
    pub tex_audit: TexAudit,
    /// `file` 指向的文件不存在（或路径不安全）时为 `true`，界面标红。
    pub damaged: bool,
}

/// 后处理档位，对应 `webwallgl` 的 `quality.postProcessing`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PostProcessing {
    High,
    #[default]
    Medium,
    Low,
    Off,
}

/// 抗锯齿档位，对应 `webwallgl` 的 `quality.antiAliasing`。
///
/// ⚠ 旧版这里是 `bool`（`antialiasing`），**前端发的是字符串 `antiAliasing`**。
/// 名字对不上 ⇒ 每次保存都被落成默认 `false` ⇒ 抗锯齿永远是关的
/// （主人 2026-11-01 报的「时钟边缘不锐利」）。
/// 这里手写 `Deserialize`：既吃新字符串，也吃旧布尔，老 `config.json` 不会解析失败。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AntiAliasing {
    #[default]
    Off,
    Fxaa,
    Msaa2,
    Msaa4,
}

impl<'de> Deserialize<'de> for AntiAliasing {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = AntiAliasing;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("off / fxaa / msaa2 / msaa4，或旧版的 true / false")
            }

            fn visit_bool<E>(self, value: bool) -> std::result::Result<AntiAliasing, E> {
                Ok(if value {
                    AntiAliasing::Fxaa
                } else {
                    AntiAliasing::Off
                })
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<AntiAliasing, E>
            where
                E: serde::de::Error,
            {
                Ok(match value.trim().to_ascii_lowercase().as_str() {
                    "fxaa" => AntiAliasing::Fxaa,
                    "msaa2" => AntiAliasing::Msaa2,
                    "msaa4" => AntiAliasing::Msaa4,
                    _ => AntiAliasing::Off,
                })
            }
        }

        deserializer.deserialize_any(Visitor)
    }
}

/// 粒子档位，对应 `webwallgl` 的 `quality.particles`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Particles {
    Off,
    Low,
    Medium,
    #[default]
    High,
}

/// 渲染档位，字段必须与前端 `WallpaperQuality`（`helpers/wallpaper.ts`）一一对应。
///
/// 默认值＝主人在 Wallpaper Engine 里选的配置：60fps / MSAA ×2 / 后处理启用(high)
/// / 纹理自动(textureScale 1) / 粒子 high。
///
/// ⚠ 这里**少一个字段就会静默丢数据**（serde 默认忽略未知字段，写盘时也没了）：
/// 2026-10-31 就是少了 `particles`、且 `antialiasing` 拼写对不上，
/// 于是「改画质 → 切壁纸 → 又变回去」，还被前端的旧默认指纹误判成「没改过」。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WallpaperQuality {
    pub fps: u32,
    pub post_processing: PostProcessing,
    /// 旧版键名是 `antialiasing`（无大写 A，且是布尔），这里用 alias 兜住。
    #[serde(alias = "antialiasing")]
    pub anti_aliasing: AntiAliasing,
    pub particles: Particles,
    pub texture_scale: f32,
}

impl Default for WallpaperQuality {
    fn default() -> Self {
        Self {
            fps: 60,
            post_processing: PostProcessing::High,
            anti_aliasing: AntiAliasing::Msaa2,
            particles: Particles::High,
            // 1.0 = native：贴图按原始分辨率装配。0.8 就是主人反复报的"壁纸像缩略图"。
            texture_scale: 1.0,
        }
    }
}

impl WallpaperQuality {
    /// 夹到安全区间：`config.json` 是用户可编辑的文件，不能信里面的数字。
    pub fn sanitized(self) -> Self {
        Self {
            fps: self.fps.clamp(1, 240),
            post_processing: self.post_processing,
            anti_aliasing: self.anti_aliasing,
            particles: self.particles,
            texture_scale: if self.texture_scale.is_finite() {
                let scale = self.texture_scale.clamp(0.25, 1.0);
                // 0.8 是旧默认值，且没有任何 UI 能选到它 —— 只可能来自老 config.json。
                // 留着它，用户升级后壁纸依旧是糊的，所以一律迁到 native。
                if (scale - 0.8).abs() < 0.01 { 1.0 } else { scale }
            } else {
                1.0
            },
        }
    }
}

/// `wallpapers/config.json` 的内容：选中项 + 属性覆盖 + 档位。
///
/// 刻意不放进 `settings` 表：那张表的 UPDATE 是几十个位置参数，加字段要同步
/// 改 SELECT 列、`$N` 编号和 bind 顺序三处（见架构文档 §5.1）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WallpaperConfig {
    /// 当前选中的包 id。
    pub selected: Option<String>,
    /// 包 id → (属性名 → 值)，用户在属性面板里改过的覆盖值。
    pub properties: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    /// 渲染档位。
    pub quality: WallpaperQuality,
}

impl WallpaperConfig {
    /// 夹一遍档位，写盘前调用。
    pub fn sanitized(mut self) -> Self {
        self.quality = self.quality.sanitized();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_project_types() {
        assert_eq!(
            WallpaperType::from_project_type("SCENE"),
            WallpaperType::Scene
        );
        assert_eq!(
            WallpaperType::from_project_type(" video\n"),
            WallpaperType::Video
        );
        assert_eq!(
            WallpaperType::from_project_type("web"),
            WallpaperType::Web
        );
        assert_eq!(
            WallpaperType::from_project_type("application"),
            WallpaperType::Application
        );
        assert_eq!(
            WallpaperType::from_project_type(""),
            WallpaperType::Other
        );
        assert_eq!(
            WallpaperType::from_project_type("plantype"),
            WallpaperType::Other
        );
    }

    #[test]
    fn only_scene_and_video_are_supported_in_phase_one() {
        assert!(WallpaperType::Scene.is_supported());
        assert!(WallpaperType::Video.is_supported());
        assert!(!WallpaperType::Web.is_supported());
        assert!(!WallpaperType::Application.is_supported());
        assert!(!WallpaperType::Other.is_supported());
    }

    #[test]
    fn quality_defaults_match_the_agreed_budget() {
        let quality = WallpaperQuality::default();

        // 主人 2026-11-01 拍板：默认值跟他 Wallpaper Engine 里的配置一致
        assert_eq!(quality.fps, 60);
        assert_eq!(quality.post_processing, PostProcessing::High);
        assert_eq!(quality.anti_aliasing, AntiAliasing::Msaa2);
        assert_eq!(quality.particles, Particles::High);
        assert!((quality.texture_scale - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn quality_is_clamped_to_safe_ranges() {
        let quality = WallpaperQuality {
            fps: 0,
            post_processing: PostProcessing::Off,
            anti_aliasing: AntiAliasing::Msaa4,
            particles: Particles::Off,
            texture_scale: f32::NAN,
        }
        .sanitized();

        assert_eq!(quality.fps, 1);
        assert_eq!(quality.post_processing, PostProcessing::Off);
        assert_eq!(quality.anti_aliasing, AntiAliasing::Msaa4);
        assert_eq!(quality.particles, Particles::Off);
        assert!((quality.texture_scale - 1.0).abs() < f32::EPSILON);

        let quality = WallpaperQuality {
            fps: 5_000,
            post_processing: PostProcessing::High,
            anti_aliasing: AntiAliasing::Off,
            particles: Particles::High,
            texture_scale: 9.0,
        }
        .sanitized();

        assert_eq!(quality.fps, 240);
        assert!((quality.texture_scale - 1.0).abs() < f32::EPSILON);
    }

    /// 回归防护（2026-11-01）：前端发的是 `antiAliasing: "msaa2"` + `particles: "high"`。
    /// 以前 Rust 侧字段名是 `antialiasing: bool`、且没有 `particles`，
    /// 于是这两个键在反序列化时被静默丢掉 ——「改画质 → 切壁纸 → 又变回去」。
    #[test]
    fn quality_keeps_every_field_the_frontend_sends() {
        let json = serde_json::json!({
            "fps": 60,
            "postProcessing": "high",
            "antiAliasing": "msaa4",
            "particles": "medium",
            "textureScale": 0.75,
        });

        let quality: WallpaperQuality = serde_json::from_value(json).expect("前端载荷必须能解析");

        assert_eq!(quality.fps, 60);
        assert_eq!(quality.post_processing, PostProcessing::High);
        assert_eq!(quality.anti_aliasing, AntiAliasing::Msaa4);
        assert_eq!(quality.particles, Particles::Medium);
        assert!((quality.texture_scale - 0.75).abs() < f32::EPSILON);

        // 序列化回去必须还是前端认识的那几个键
        let back = serde_json::to_value(quality).expect("必须能序列化");
        assert_eq!(back["antiAliasing"], serde_json::json!("msaa4"));
        assert_eq!(back["particles"], serde_json::json!("medium"));
    }

    /// 老 `config.json` 里是 `antialiasing: false`（布尔），不能因为改名就整个解析失败
    /// —— 解析失败会退化成 `WallpaperConfig::default()`，把用户选中的壁纸一起丢掉。
    #[test]
    fn legacy_boolean_antialiasing_still_parses() {
        let off: WallpaperQuality =
            serde_json::from_value(serde_json::json!({ "antialiasing": false, "fps": 30 })).unwrap();
        assert_eq!(off.anti_aliasing, AntiAliasing::Off);
        assert_eq!(off.fps, 30);
        // 缺的字段走 Default
        assert_eq!(off.particles, Particles::High);

        let on: WallpaperQuality =
            serde_json::from_value(serde_json::json!({ "antialiasing": true })).unwrap();
        assert_eq!(on.anti_aliasing, AntiAliasing::Fxaa);
    }

    #[test]
    fn config_round_trips_through_json_with_camel_case_keys() {
        let mut config = WallpaperConfig::default();
        config.selected = Some("local-river-0123456789ab".to_string());
        config.properties.insert(
            "local-river-0123456789ab".to_string(),
            BTreeMap::from([(
                "speed".to_string(),
                serde_json::json!(1.5),
            )]),
        );

        let text = serde_json::to_string(&config).expect("serialise config");
        assert!(text.contains("\"selected\""));
        assert!(text.contains("\"postProcessing\""));

        let parsed: WallpaperConfig =
            serde_json::from_str(&text).expect("parse config");
        assert_eq!(parsed, config);
    }

    #[test]
    fn config_tolerates_missing_and_partial_json() {
        let parsed: WallpaperConfig =
            serde_json::from_str("{}").expect("empty object is valid");
        assert_eq!(parsed, WallpaperConfig::default());

        let parsed: WallpaperConfig = serde_json::from_str(
            r#"{"selected":"local-x-000000000000","quality":{"fps":60}}"#,
        )
        .expect("partial quality is valid");
        assert_eq!(parsed.quality.fps, 60);
        // 只给了 fps，其余字段走 Default（= 主人 WE 那套：high / msaa2 / high / 1.0）
        assert_eq!(
            parsed.quality.post_processing,
            WallpaperQuality::default().post_processing
        );
        assert_eq!(
            parsed.quality.anti_aliasing,
            WallpaperQuality::default().anti_aliasing
        );
        assert_eq!(
            parsed.quality.particles,
            WallpaperQuality::default().particles
        );
    }

    #[test]
    fn etc2_audit_produces_a_warning() {
        let audit = TexAudit {
            etc2: true,
            total: 3,
            ..TexAudit::default()
        };

        let warning = audit.warning().expect("etc2 should warn");
        assert!(warning.contains("ETC2"));
    }

    #[test]
    fn clean_audit_produces_no_warning() {
        let audit = TexAudit {
            dxt: true,
            total: 4,
            ..TexAudit::default()
        };

        assert!(audit.warning().is_none());
    }

    #[test]
    fn error_messages_are_single_sentence_chinese_without_paths() {
        let error = WallpaperError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "C:\\Users\\someone\\secret.txt",
        ));

        let message = error.user_message();
        assert_eq!(message, "读写壁纸文件失败");
        assert!(!message.contains("C:"));
        assert!(!message.contains("secret"));
    }
}
