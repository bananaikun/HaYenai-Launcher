//! AI Agent 面板：工具驱动的实例助手。
//!
//! 循环由前端驱动（turn → tool_calls → 执行 → 回灌 → turn），
//! 与 LogAgent 同构 —— 好处是"思考过程"能一步步画出来，而不是在 Rust 里闷头跑完。
//!
//! **权限只有两档：只读 / 可写。** 只读档下写工具根本不出现在工具清单里，
//! 模型看不到也就不会去调 —— 比看到了再拒绝更省一次往返，也少一次"它明明能看见却做不了"的困惑。
//!
//! 刻意**不提供**发 MC 命令的工具：启动器侧连游戏进程要通过 RCON 或日志注入，
//! 两条路都会让"AI 操作不当"变成"存档被改坏"。实例层面改名字、改内存已经够用。

use crate::api::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use theseus::ai::chat::{ChatMessage, ChatModel, ChatTurn};
use theseus::ai_preset;

/// 单次读文件上限。超过就截断 —— Agent 要的是"看一眼"，不是把 jar 灌进上下文。
const MAX_READ_BYTES: u64 = 64 * 1024;
/// 单次读日志上限（尾部行数）。
const MAX_LOG_LINES: usize = 400;
/// 列目录上限，防止 assets/ 这种几千条的目录把上下文撑爆。
const MAX_DIR_ENTRIES: usize = 200;

#[derive(Clone, Debug, Serialize)]
pub struct AgentEvent {
    /// `delta` = 正文增量；`tool` = 将要调用的工具名（前端用来提前出卡片）
    pub kind: String,
    pub text: String,
}

/* ------------------------------------------------------------------ *
 * 工具清单
 * ------------------------------------------------------------------ */

pub fn tool_definitions(allow_write: bool) -> Vec<Value> {
    let mut tools = vec![
        tool(
            "list_instances",
            "列出启动器里的所有实例。先用它确认用户在说哪一个，不要凭名字猜。",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "get_instance",
            "取一个实例的完整信息：版本、加载器、路径、分组、内存、最近游玩时间。",
            json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string", "description": "实例 ID（来自 list_instances）" }
                },
                "required": ["instance_id"],
                "additionalProperties": false
            }),
        ),
        tool(
            "read_log",
            "读实例最近的日志尾部。排查崩溃、模组冲突、启动失败时先看它。",
            json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string" },
                    "filename": {
                        "type": "string",
                        "description": "日志文件名。省略则读最新的一份。"
                    },
                    "lines": {
                        "type": "integer",
                        "description": "要尾部多少行，默认 200，上限 400。"
                    }
                },
                "required": ["instance_id"],
                "additionalProperties": false
            }),
        ),
        tool(
            "list_files",
            "列出实例目录下的文件。path 是相对实例根目录的路径，空字符串表示根目录。",
            json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string" },
                    "path": { "type": "string", "description": "相对路径，如 config 或 mods" }
                },
                "required": ["instance_id"],
                "additionalProperties": false
            }),
        ),
        tool(
            "read_file",
            "读实例目录内的一个文本文件（配置、崩溃报告等）。最大 64 KB，二进制文件会被拒绝。",
            json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string" },
                    "path": { "type": "string", "description": "相对实例根目录的文件路径" },
                    "max_bytes": { "type": "integer", "description": "最多读多少字节，默认 65536" }
                },
                "required": ["instance_id", "path"],
                "additionalProperties": false
            }),
        ),
    ];

    if allow_write {
        tools.push(tool(
            "rename_instance",
            "给实例改名。改动立即生效、可撤销（再改回来即可），但要先跟用户确认新名字。",
            json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string" },
                    "name": { "type": "string", "description": "新名字，1-64 个字符" }
                },
                "required": ["instance_id", "name"],
                "additionalProperties": false
            }),
        ));
        tools.push(tool(
            "set_memory",
            "改实例的内存分配（MB）。下线 512，上限 32768。改之前必须告诉用户具体数值。",
            json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string" },
                    "megabytes": { "type": "integer", "description": "内存上限，单位 MB" }
                },
                "required": ["instance_id", "megabytes"],
                "additionalProperties": false
            }),
        ));
    }

    tools
}

fn tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": { "name": name, "description": description, "parameters": parameters }
    })
}

/* ------------------------------------------------------------------ *
 * 工具执行
 * ------------------------------------------------------------------ */

pub async fn run_tool(
    name: &str,
    args: Value,
    allow_write: bool,
) -> Result<Value> {
    match name {
        "list_instances" => list_instances().await,
        "get_instance" => get_instance(&string_arg(&args, "instance_id")?).await,
        "read_log" => {
            read_log(
                &string_arg(&args, "instance_id")?,
                args.get("filename").and_then(Value::as_str),
                args.get("lines").and_then(Value::as_u64).unwrap_or(200),
            )
            .await
        }
        "list_files" => {
            list_files(
                &string_arg(&args, "instance_id")?,
                args.get("path").and_then(Value::as_str).unwrap_or(""),
            )
            .await
        }
        "read_file" => {
            read_file(
                &string_arg(&args, "instance_id")?,
                &string_arg(&args, "path")?,
                args.get("max_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(MAX_READ_BYTES),
            )
            .await
        }
        "rename_instance" => {
            require_write(allow_write)?;
            rename_instance(
                &string_arg(&args, "instance_id")?,
                &string_arg(&args, "name")?,
            )
            .await
        }
        "set_memory" => {
            require_write(allow_write)?;
            let megabytes = args
                .get("megabytes")
                .and_then(Value::as_u64)
                .ok_or_else(|| bad_args("缺少 megabytes"))?;
            set_memory(&string_arg(&args, "instance_id")?, megabytes).await
        }
        other => Err(bad_args(format!("未知工具：{other}"))),
    }
}

/// 写工具在只读档下被调到时，报错要能让人看懂发生了什么。
fn require_write(allow_write: bool) -> Result<()> {
    if allow_write {
        return Ok(());
    }
    Err(bad_args("只读模式下不允许修改。请在面板上打开「允许修改」，再让 Agent 执行。"
                .to_string(),
    ))
}

fn bad_args(message: impl Into<String>) -> crate::api::TheseusSerializableError {
    theseus::Error::from(theseus::ErrorKind::InputError(message.into())).into()
}

fn string_arg(args: &Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| bad_args(format!("缺少参数 {key}")))
}

async fn list_instances() -> Result<Value> {
    let instances = theseus::instance::list().await?;
    let rows = instances
        .iter()
        .map(|meta| {
            json!({
                "instance_id": meta.instance.id,
                "name": meta.instance.name,
                "game_version": game_version_of(meta),
                "install_stage": meta.instance.install_stage,
                "groups": meta.groups,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({ "count": rows.len(), "instances": rows }))
}

async fn get_instance(instance_id: &str) -> Result<Value> {
    let meta = theseus::instance::get(instance_id)
        .await?
        .ok_or_else(|| bad_args(format!("实例不存在：{instance_id}")))?;
    Ok(json!({
        "instance_id": meta.instance.id,
        "name": meta.instance.name,
        "game_version": game_version_of(&meta),
        "loader_components": meta.loader_components,
        "path": meta.instance.path,
        "install_stage": meta.instance.install_stage,
        "groups": meta.groups,
        "memory": meta.launch_overrides.memory,
        "java_path": meta.launch_overrides.java_path,
        "last_played": meta.instance.last_played,
    }))
}

/// `Instance` 上的版本信息按加载器不同散在两处，统一成一个字段给模型看。
fn game_version_of(meta: &theseus::state::InstanceMetadata) -> String {
    meta.loader_components
        .iter()
        .find(|component| {
            matches!(component.kind, theseus::state::LoaderComponentKind::Vanilla)
        })
        .and_then(|component| component.version.clone())
        .unwrap_or_default()
}

async fn read_log(
    instance_id: &str,
    filename: Option<&str>,
    lines: u64,
) -> Result<Value> {
    let logs = theseus::logs::get_logs(instance_id, None).await?;
    let picked = match filename {
        Some(name) => logs.into_iter().find(|log| log.filename == name),
        None => logs.into_iter().max_by_key(|log| log.age),
    };
    let Some(log) = picked else {
        return Ok(json!({ "found": false, "reason": "这个实例还没有日志" }));
    };
    let content = log
        .output
        .as_ref()
        .map(|value| value.as_str().to_string())
        .unwrap_or_default();
    let limit = (lines as usize).clamp(1, MAX_LOG_LINES);
    let tail = tail_lines(&content, limit);
    Ok(json!({
        "found": true,
        "filename": log.filename,
        "total_lines": content.lines().count(),
        "returned_lines": tail.lines().count(),
        "content": tail,
    }))
}

fn tail_lines(text: &str, limit: usize) -> String {
    let collected = text.lines().collect::<Vec<_>>();
    let start = collected.len().saturating_sub(limit);
    collected[start..].join("\n")
}

async fn list_files(instance_id: &str, relative: &str) -> Result<Value> {
    let root = instance_root(instance_id).await?;
    let target = safe_join(&root, relative)?;
    let mut entries = Vec::new();
    let mut reader = tokio::fs::read_dir(&target).await?;
    while let Some(entry) = reader.next_entry().await? {
        if entries.len() >= MAX_DIR_ENTRIES {
            break;
        }
        let meta = entry.metadata().await.ok();
        let name = entry.file_name().to_string_lossy().to_string();
        entries.push(json!({
            "name": name,
            "dir": meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
            "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
        }));
    }
    entries.sort_by(|a, b| {
        let a_dir = a["dir"].as_bool().unwrap_or(false);
        let b_dir = b["dir"].as_bool().unwrap_or(false);
        b_dir.cmp(&a_dir).then_with(|| {
            a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or(""))
        })
    });
    Ok(json!({ "path": relative, "entries": entries, "truncated": entries.len() >= MAX_DIR_ENTRIES }))
}

async fn read_file(
    instance_id: &str,
    relative: &str,
    max_bytes: u64,
) -> Result<Value> {
    let root = instance_root(instance_id).await?;
    let target = safe_join(&root, relative)?;
    let meta = tokio::fs::metadata(&target).await?;
    if !meta.is_file() {
        return Err(bad_args(format!("不是文件：{relative}")));
    }
    let limit = max_bytes.clamp(1, MAX_READ_BYTES);
    let bytes = tokio::fs::read(&target).await?;
    let truncated = bytes.len() as u64 > limit;
    let slice = &bytes[..bytes.len().min(limit as usize)];
    let text = match String::from_utf8(slice.to_vec()) {
        Ok(text) => text,
        Err(_) => {
            return Err(bad_args(format!(
                "{relative} 不是 UTF-8 文本（可能是二进制文件），读它没有意义"
            )))
        }
    };
    Ok(json!({ "path": relative, "content": text, "truncated": truncated, "size": bytes.len() }))
}

async fn rename_instance(instance_id: &str, name: &str) -> Result<Value> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 64 {
        return Err(bad_args("名字要 1-64 个字符"));
    }
    let patch: theseus::state::EditInstance =
        serde_json::from_value(json!({ "name": trimmed }))
            .map_err(|error| bad_args(format!("构造补丁失败：{error}")))?;
    let updated = theseus::instance::edit(instance_id, patch).await?;
    Ok(json!({ "ok": true, "instance_id": updated.instance.id, "name": updated.instance.name }))
}

async fn set_memory(instance_id: &str, megabytes: u64) -> Result<Value> {
    if !(512..=32768).contains(&megabytes) {
        return Err(bad_args("内存要在 512-32768 MB 之间"));
    }
    let patch: theseus::state::EditInstance = serde_json::from_value(json!({
        "launch_overrides": {
            "memory": { "maximum": megabytes as u32, "automatic": false, "optimize_before_launch": false }
        }
    }))
    .map_err(|error| bad_args(format!("构造补丁失败：{error}")))?;
    let updated = theseus::instance::edit(instance_id, patch).await?;
    Ok(json!({
        "ok": true,
        "instance_id": updated.instance.id,
        "memory_mb": updated.launch_overrides.memory.map(|m| m.maximum),
    }))
}

async fn instance_root(instance_id: &str) -> Result<PathBuf> {
    let meta = theseus::instance::get(instance_id)
        .await?
        .ok_or_else(|| bad_args(format!("实例不存在：{instance_id}")))?;
    Ok(PathBuf::from(meta.instance.path))
}

/// 把相对路径拼到实例根目录上，并确认**结果仍在根目录内**。
///
/// 只 `canonicalize` 目标是不够的：`../../.ssh/id_rsa` 会被规范化成一个合法路径。
/// 必须先拒绝 `..` 与绝对路径，再在存在的前提下规范化比对前缀 —— 两者都做才拦得住
/// 「用符号链接跳出去」这种更隐蔽的写法。
fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let trimmed = relative.trim();
    if trimmed.is_empty() {
        return Ok(root.to_path_buf());
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Err(bad_args("路径必须是相对实例目录的"));
    }
    for component in candidate.components() {
        if matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)) {
            return Err(bad_args("路径里不能出现 .."));
        }
    }
    let joined = root.join(candidate);
    let root_real = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let joined_real = joined.canonicalize().unwrap_or_else(|_| joined.clone());
    if !joined_real.starts_with(&root_real) {
        return Err(bad_args("路径跑到实例目录外面了"));
    }
    Ok(joined_real)
}

/* ------------------------------------------------------------------ *
 * 一轮对话
 * ------------------------------------------------------------------ */

#[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
pub struct AgentTurnRequest {
    /// 方案 ID。给了就用它的 provider/model/人格/规则。
    pub preset_id: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    /// 面板的「允许修改」开关。
    #[serde(default)]
    pub allow_write: bool,
    pub messages: Vec<ChatMessage>,
}

/// 跑一轮。前端拿到 `tool_calls` 后自己调 `agent_tool` 执行，再带着结果回来。
pub async fn turn(
    request: AgentTurnRequest,
    emit: impl Fn(AgentEvent) + Sync,
) -> Result<ChatMessage> {
    let preset = match request.preset_id.as_deref() {
        Some(id) if !id.is_empty() => Some(
            ai_preset::find(id)
                .await?
                .ok_or_else(|| bad_args("方案不存在，可能已被删除"))?,
        ),
        _ => None,
    };

    let provider_id = preset
        .as_ref()
        .map(|p| p.provider_id.clone())
        .filter(|value| !value.is_empty())
        .or(request.provider_id)
        .ok_or_else(|| bad_args("还没选模型。到 Agent 页的「模型」下拉里挑一个。"))?;
    let model_id = preset
        .as_ref()
        .map(|p| p.model_id.clone())
        .filter(|value| !value.is_empty())
        .or(request.model_id)
        .ok_or_else(|| bad_args("还没选模型。到 Agent 页的「模型」下拉里挑一个。"))?;

    let mut messages = request.messages;
    // 人格/规则永远排第一，且不让前端传的 system 覆盖掉 —— 方案是唯一真源。
    let persona = match &preset {
        Some(preset) => ai_preset::system_prompt(preset),
        None => String::new(),
    };
    let base = AGENT_PREAMBLE.trim();
    let system = if persona.is_empty() {
        base.to_string()
    } else {
        format!("{persona}\n\n{base}")
    };
    messages.insert(
        0,
        ChatMessage {
            role: "system".to_string(),
            content: system,
            tool_calls: Vec::new(),
            tool_call_id: None,
            opaque: Value::Null,
            attachments: Vec::new(),
        },
    );

    let model = ChatModel::prepare(&provider_id, &model_id).await?;
    let message = model
        .turn(
            ChatTurn {
                messages,
                tools: tool_definitions(request.allow_write),
            },
            &|kind, text| {
                emit(AgentEvent {
                    kind: kind.to_string(),
                    text: text.to_string(),
                });
            },
        )
        .await?;
    Ok(message)
}

/// 写死在代码里的一段，和人格/规则分开：它是「怎么用工具」的说明书，
/// 不是「你扮演谁」的设定 —— 用户改人格时不该顺手把它删了。
const AGENT_PREAMBLE: &str = "\
你在启动器内部的 AI 面板里工作，能读用户的实例、日志和配置文件，被允许时也能改实例名和内存。

规则：
- 涉及具体实例时先用 list_instances / get_instance 确认，不要凭用户口语里的名字猜。
- 排查问题时先 read_log 看真实日志，再说结论；没有证据就直说「日志里看不出来」。
- 需要改动时先讲清楚要改什么、改成什么，再调工具。不确定时问，不要自己决定。
- 回答用中文，简短，不要复述工具返回的原文。";

/* ------------------------------------------------------------------ *
 * Tauri 命令
 * ------------------------------------------------------------------ */

#[tauri::command]
pub async fn agent_tools(allow_write: bool) -> Result<Value> {
    Ok(Value::Array(tool_definitions(allow_write)))
}

#[tauri::command]
pub async fn agent_run_tool(
    name: String,
    args: Value,
    allow_write: bool,
) -> Result<Value> {
    run_tool(&name, args, allow_write).await
}

#[tauri::command]
pub async fn agent_turn(
    request: AgentTurnRequest,
    on_event: tauri::ipc::Channel<AgentEvent>,
) -> Result<ChatMessage> {
    turn(request, |event| {
        let _ = on_event.send(event);
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_read_tools_without_write_permission() {
        let names = tool_definitions(false)
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(names.contains(&"read_log".to_string()));
        assert!(!names.contains(&"rename_instance".to_string()));
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn write_tools_appear_only_when_allowed() {
        assert_eq!(tool_definitions(true).len(), 7);
    }

    #[test]
    fn parent_dir_is_rejected() {
        let root = std::env::temp_dir();
        assert!(safe_join(&root, "../secret").is_err());
        assert!(safe_join(&root, "a/../../b").is_err());
    }

    #[test]
    fn absolute_path_is_rejected() {
        let root = std::env::temp_dir();
        let absolute = if cfg!(windows) { "C:\\Windows" } else { "/etc" };
        assert!(safe_join(&root, absolute).is_err());
    }

    #[test]
    fn nested_relative_path_is_allowed() {
        let root = std::env::temp_dir();
        assert!(safe_join(&root, "config/options.txt").is_ok());
    }

    #[test]
    fn tail_lines_keeps_the_end() {
        let text = (1..=10)
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(tail_lines(&text, 3), "8\n9\n10");
        assert_eq!(tail_lines(&text, 100).lines().count(), 10);
    }
}
