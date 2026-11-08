use crate::api::Result;
use theseus::ai::{
    self, AiModelUpdate, AiProviderConfigUpdate, AiProviderDefinition,
    AiProviderModel, AiSettings, AiState, OAuthDeviceCode, OAuthPollStatus,
};
use theseus::ai_preset::{self, AiPreset, AiPresetInput, AiPresetState};

pub fn init<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("ai")
        .invoke_handler(tauri::generate_handler![
            ai_get_catalog,
            ai_get_state,
            ai_update_settings,
            ai_update_provider,
            ai_set_api_key,
            ai_set_credential,
            ai_update_model,
            ai_remove_model,
            ai_fetch_models,
            ai_test_provider,
            ai_begin_oauth,
            ai_poll_oauth,
            ai_disconnect_oauth,
        ])
        .build()
}

#[tauri::command]
pub fn ai_get_catalog() -> Vec<AiProviderDefinition> {
    ai::catalog()
}

#[tauri::command]
pub async fn ai_get_state() -> Result<AiState> {
    Ok(ai::get_state().await?)
}

#[tauri::command]
pub async fn ai_update_settings(settings: AiSettings) -> Result<()> {
    Ok(ai::update_settings(settings).await?)
}

#[tauri::command]
pub async fn ai_update_provider(update: AiProviderConfigUpdate) -> Result<()> {
    Ok(ai::update_provider(update).await?)
}

#[tauri::command]
pub fn ai_set_api_key(
    provider_id: String,
    secret: Option<String>,
) -> Result<()> {
    Ok(ai::set_api_key(provider_id, secret)?)
}

#[tauri::command]
pub fn ai_set_credential(
    provider_id: String,
    credential: String,
    secret: Option<String>,
) -> Result<()> {
    Ok(ai::set_credential(provider_id, credential, secret)?)
}

#[tauri::command]
pub async fn ai_update_model(update: AiModelUpdate) -> Result<()> {
    Ok(ai::update_model(update).await?)
}

#[tauri::command]
pub async fn ai_remove_model(
    provider_id: String,
    model_id: String,
) -> Result<()> {
    Ok(ai::remove_model(provider_id, model_id).await?)
}

#[tauri::command]
pub async fn ai_fetch_models(
    provider_id: String,
) -> Result<Vec<AiProviderModel>> {
    Ok(ai::fetch_models(provider_id).await?)
}

#[tauri::command]
pub async fn ai_test_provider(
    provider_id: String,
    model_id: String,
) -> Result<String> {
    Ok(ai::test_provider(provider_id, model_id).await?)
}

#[tauri::command]
pub async fn ai_begin_oauth(provider_id: String) -> Result<OAuthDeviceCode> {
    Ok(ai::begin_oauth(provider_id).await?)
}

#[tauri::command]
pub async fn ai_poll_oauth(flow_id: uuid::Uuid) -> Result<OAuthPollStatus> {
    Ok(ai::poll_oauth(flow_id).await?)
}

#[tauri::command]
pub fn ai_disconnect_oauth(provider_id: String) -> Result<()> {
    Ok(ai::disconnect_oauth(provider_id)?)
}

/* ------------------------------------------------------------------ *
 * 方案（Preset）
 *
 * 人格 + 规则 + 绑定的厂商/模型。机器人、Agent 都读 `active_preset_id`，
 * 不必各自维护一份人格配置。
 *
 * **注册在主 `invoke_handler` 里，不走 ai 插件**：插件通道的 ACL 权限清单是
 * 编译期快照（`gen/schemas/acl-manifests.json`），加命令后即使重编也仍是旧表，
 * 调用会得到 `ai.ai_preset_list not allowed. Command not found`。
 * 主 handler 的命令不需要单独授权，与 `wallpaper_*` 一致。
 * ------------------------------------------------------------------ */

#[tauri::command]
pub async fn ai_preset_list() -> Result<AiPresetState> {
    Ok(ai_preset::list().await?)
}

#[tauri::command]
pub async fn ai_preset_save(input: AiPresetInput) -> Result<AiPreset> {
    Ok(ai_preset::save(input).await?)
}

#[tauri::command]
pub async fn ai_preset_remove(id: String) -> Result<()> {
    Ok(ai_preset::remove(&id).await?)
}

#[tauri::command]
pub async fn ai_preset_set_active(id: Option<String>) -> Result<()> {
    Ok(ai_preset::set_active(id.as_deref()).await?)
}

/// 用某个方案发一句话。用来验证人格与规则是否真的生效。
#[tauri::command]
pub async fn ai_preset_complete(
    preset_id: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    user_prompt: String,
) -> Result<String> {
    Ok(ai_preset::complete(
        &preset_id,
        provider_id.as_deref(),
        model_id.as_deref(),
        &user_prompt,
    )
    .await?)
}
