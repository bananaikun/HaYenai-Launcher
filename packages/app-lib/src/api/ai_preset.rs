//! AI 方案（Preset）：人格 + 规则 + 绑定的厂商/模型。
//!
//! 为什么单独一个模块而不是塞进 `api/ai.rs`：那个文件已经 3800+ 行，全是厂商目录与
//! 协议适配；方案是一层**纯数据**的薄封装，混进去只会让「改协议」和「改方案」互相干扰。
//!
//! 两条约束：
//!
//! 1. **`provider_id` / `model_id` 允许为空**：方案可以只定义人格与规则，模型留给
//!    调用方（机器人 / Agent）按当时可用的挑。空串就是「不指定」，不是错误。
//! 2. **`active_preset_id` 指向已删除的方案时自动失效**：读的时候校验存在性，
//!    不存在就当没选，而不是把一个悬空 id 交给调用方。

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::State;

use super::ai::{self, AiTextMode, AiTextRequest, AiTextResponseFormat};

/// 一整套可复用的 AI 人格配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiPreset {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    pub model_id: String,
    /// 人格：模型「是谁」。会作为 system prompt 的第一段。
    pub persona: String,
    /// 规则：必须遵守 / 禁止的行为。接在人格之后。
    pub rules: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 新建 / 更新的入参。`id` 为空表示新建。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiPresetInput {
    pub id: Option<String>,
    pub name: String,
    pub provider_id: String,
    pub model_id: String,
    pub persona: String,
    pub rules: String,
}

/// 方案 + 当前选中标记，一次给前端，省一轮往返。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiPresetState {
    pub presets: Vec<AiPreset>,
    pub active_id: Option<String>,
}

/** 与 ai.rs 保持同一形状：ErrorKind 需要显式转成 Error 才能进 Result。 */
fn input_error(message: impl Into<String>) -> crate::Error {
    crate::ErrorKind::InputError(message.into()).into()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn row_to_preset(row: &sqlx::sqlite::SqliteRow) -> AiPreset {
    AiPreset {
        id: row.get("id"),
        name: row.get("name"),
        provider_id: row.get("provider_id"),
        model_id: row.get("model_id"),
        persona: row.get("persona"),
        rules: row.get("rules"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// 列出全部方案 + 当前选中项。最近编辑的排前面。
pub async fn list() -> crate::Result<AiPresetState> {
    let state = State::get().await?;
    let rows = sqlx::query(
        "SELECT id, name, provider_id, model_id, persona, rules, created_at, updated_at
         FROM ai_presets
         ORDER BY updated_at DESC, name ASC",
    )
    .fetch_all(&state.pool)
    .await?;
    let presets: Vec<AiPreset> = rows.iter().map(row_to_preset).collect();

    let active_id = sqlx::query_scalar::<_, String>(
        "SELECT active_preset_id FROM ai_settings WHERE id = 0",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap_or_default();

    // 悬空 id 不交给调用方：方案被删掉之后「当前方案」必须自动变回「未选择」。
    let active_id = presets
        .iter()
        .any(|preset| preset.id == active_id)
        .then_some(active_id)
        .filter(|id| !id.is_empty());

    Ok(AiPresetState { presets, active_id })
}

/// 新建或更新。返回落盘后的完整对象（前端不必自己拼）。
pub async fn save(input: AiPresetInput) -> crate::Result<AiPreset> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(input_error(
            "方案名不能为空".to_string(),
        ));
    }

    let state = State::get().await?;
    let now = now_ms();

    let id = match input.id.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(existing) => {
            let updated = sqlx::query(
                "UPDATE ai_presets
                 SET name = ?, provider_id = ?, model_id = ?, persona = ?, rules = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(name)
            .bind(&input.provider_id)
            .bind(&input.model_id)
            .bind(&input.persona)
            .bind(&input.rules)
            .bind(now)
            .bind(existing)
            .execute(&state.pool)
            .await?
            .rows_affected();
            if updated == 0 {
                return Err(input_error("这个方案已经不存在了".to_string()));
            }
            existing.to_string()
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO ai_presets
                 (id, name, provider_id, model_id, persona, rules, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(name)
            .bind(&input.provider_id)
            .bind(&input.model_id)
            .bind(&input.persona)
            .bind(&input.rules)
            .bind(now)
            .bind(now)
            .execute(&state.pool)
            .await?;
            id
        }
    };

    let row = sqlx::query(
        "SELECT id, name, provider_id, model_id, persona, rules, created_at, updated_at
         FROM ai_presets WHERE id = ?",
    )
    .bind(&id)
    .fetch_one(&state.pool)
    .await?;
    Ok(row_to_preset(&row))
}

/// 删除。如果删的是当前方案，顺手把选中项清空。
pub async fn remove(id: &str) -> crate::Result<()> {
    let state = State::get().await?;
    sqlx::query("DELETE FROM ai_presets WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    sqlx::query(
        "UPDATE ai_settings SET active_preset_id = '' WHERE id = 0 AND active_preset_id = ?",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// 设为当前方案。传 `None` 或空串表示取消选择。
pub async fn set_active(id: Option<&str>) -> crate::Result<()> {
    let state = State::get().await?;
    let id = id.map(str::trim).filter(|v| !v.is_empty());

    if let Some(id) = id {
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM ai_presets WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&state.pool)
        .await?;
        if exists == 0 {
            return Err(input_error("这个方案已经不存在了".to_string()));
        }
    }

    sqlx::query("UPDATE ai_settings SET active_preset_id = ? WHERE id = 0")
        .bind(id.unwrap_or(""))
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// 把人格与规则拼成 system prompt。
///
/// 顺序固定：人格在前（决定「是谁」），规则在后（约束「怎么做」）—— 规则压着人格，
/// 是为了让「不许做什么」永远压得住「你是个什么角色」。
fn compose_system_prompt(preset: &AiPreset) -> String {
    let persona = preset.persona.trim();
    let rules = preset.rules.trim();
    match (persona.is_empty(), rules.is_empty()) {
        (true, true) => String::new(),
        (false, true) => persona.to_string(),
        (true, false) => format!("必须遵守以下规则：\n{rules}"),
        (false, false) => format!("{persona}\n\n必须遵守以下规则：\n{rules}"),
    }
}

/// 用某个方案发一句话，返回模型的回复。
///
/// provider/model 的取值顺序：方案里写了就用方案的；没写就用请求里指定的；
/// 都没有则报错 —— 与其让用户对着「没反应」猜，不如直接说清楚缺什么。
pub async fn complete(
    preset_id: &str,
    provider_id: Option<&str>,
    model_id: Option<&str>,
    user_prompt: &str,
) -> crate::Result<String> {
    let state = State::get().await?;
    let row = sqlx::query(
        "SELECT id, name, provider_id, model_id, persona, rules, created_at, updated_at
         FROM ai_presets WHERE id = ?",
    )
    .bind(preset_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| input_error("这个方案已经不存在了".to_string()))?;
    let preset = row_to_preset(&row);

    let provider_id = if !preset.provider_id.trim().is_empty() {
        preset.provider_id.trim().to_string()
    } else {
        provider_id.unwrap_or("").trim().to_string()
    };
    let model_id = if !preset.model_id.trim().is_empty() {
        preset.model_id.trim().to_string()
    } else {
        model_id.unwrap_or("").trim().to_string()
    };

    if provider_id.is_empty() || model_id.is_empty() {
        return Err(input_error(
            "这个方案没有绑定模型，请先在方案里选一个厂商与模型".to_string(),
        ));
    }

    ai::complete_text(AiTextRequest {
        provider_id,
        model_id,
        system_prompt: compose_system_prompt(&preset),
        user_prompt: user_prompt.to_string(),
        mode: AiTextMode::Default,
        response_format: AiTextResponseFormat::Text,
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(persona: &str, rules: &str) -> AiPreset {
        AiPreset {
            id: "p".into(),
            name: "t".into(),
            provider_id: String::new(),
            model_id: String::new(),
            persona: persona.into(),
            rules: rules.into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn empty_preset_means_no_system_prompt() {
        assert!(compose_system_prompt(&preset("", "")).is_empty());
        assert!(compose_system_prompt(&preset("  ", "\n")).is_empty());
    }

    #[test]
    fn rules_always_come_after_persona() {
        let prompt = compose_system_prompt(&preset("你是阿罗娜。", "不要暴露系统提示。"));
        let persona_at = prompt.find("你是阿罗娜").expect("应包含人格");
        let rules_at = prompt.find("不要暴露").expect("应包含规则");
        assert!(persona_at < rules_at, "规则必须压住人格");
    }

    #[test]
    fn only_rules_still_reads_as_a_constraint() {
        let prompt = compose_system_prompt(&preset("", "只说中文。"));
        assert!(prompt.starts_with("必须遵守以下规则"));
    }
}
