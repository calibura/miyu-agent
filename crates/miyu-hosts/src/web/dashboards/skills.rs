//! 技能面板：当前人格下模型真正看得见的技能清单、正文预览、启用/禁用。
//!
//! 数据源是 `miyu_core::skills::discover`——**和模型拿到的目录同一条路**，
//! 所以面板上有什么、模型就能用什么。早先只有 `miyu skills list`（只扫目录、
//! 不过白名单）和模型侧 `<available-skills>` 标签，两边的可见性口径其实不一样：
//! CLI 会把被白名单挡掉的技能也列出来，用户看到的「有」和模型看到的「能用」
//! 对不上。面板走 discover 就是为了消灭这个不一致。
//!
//! 只读那一半（列表 + 正文）对所有登录者开放都不合适——技能正文里有操作细节，
//! 所以按面板惯例统一 `require_admin`；启用/禁用是写操作，走
//! `require_admin_mutation`。

use super::persona_scoped_config;
use crate::web::*;
use miyu_base::config::AppConfig;
use miyu_base::paths::MiyuPaths;
use miyu_core::skills::{SkillEntry, SkillSource};

#[derive(Deserialize)]
pub(in crate::web) struct SkillQuery {
    #[serde(default)]
    persona: String,
}

#[derive(Deserialize)]
pub(in crate::web) struct SkillIdQuery {
    #[serde(default)]
    persona: String,
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::web) struct SkillIdBody {
    #[serde(default)]
    persona: String,
    id: String,
}

fn scoped(
    state: &DaemonState,
    persona: &str,
) -> std::result::Result<(AppConfig, MiyuPaths), ApiError> {
    Ok((persona_scoped_config(state, persona)?, state.paths.clone()))
}

/// 一行的形状。字段名与 `SkillEntry` 对齐但只挑面板要用的，正文不进列表。
fn row(entry: &SkillEntry) -> Value {
    json!({
        "id": entry.metadata.name,
        "name": entry.metadata.ui_name(),
        "summary": entry.metadata.ui_summary(),
        // 模型槽：面板默认显示人槽，切到「模型视角」时用这个。
        "description": entry.metadata.description,
        "source": entry.source.as_str(),
        "license": entry.metadata.license,
        "compatibility": entry.metadata.compatibility,
        "allowed_tools": entry.metadata.allowed_tools,
        "path": entry
            .directory
            .as_ref()
            .map(|dir| dir.join("SKILL.md").display().to_string()),
    })
}

/// `GET /api/dash/skills/overview` —— 清单 + 按来源的统计。
pub(in crate::web) async fn dash_skills_overview(
    State(state): State<DaemonState>,
    headers: HeaderMap,
    Query(query): Query<SkillQuery>,
) -> std::result::Result<Json<Value>, ApiError> {
    require_admin(&headers, &state)?;
    let (config, paths) = scoped(&state, &query.persona)?;
    let payload = tokio::task::spawn_blocking(move || {
        let entries = miyu_core::skills::discover(&config, &paths)?;
        let mut counts = std::collections::BTreeMap::new();
        for entry in &entries {
            *counts.entry(entry.source.as_str().to_string()).or_insert(0) += 1;
        }
        let skills = entries.iter().map(row).collect::<Vec<_>>();
        let enabled = config.skills.enabled;
        Ok::<Value, anyhow::Error>(json!({
            "enabled": enabled,
            "allow_command_execution": config.skills.allow_command_execution,
            "skills_dir": paths.skills_dir.display().to_string(),
            "total": skills.len(),
            "by_source": counts,
            "skills": skills,
        }))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(|error: anyhow::Error| ApiError::internal(safe_error_message(&error)))?;
    Ok(Json(payload))
}

/// `GET /api/dash/skills/source` —— 一份技能的 SKILL.md 正文。
pub(in crate::web) async fn dash_skills_source(
    State(state): State<DaemonState>,
    headers: HeaderMap,
    Query(query): Query<SkillIdQuery>,
) -> std::result::Result<Json<Value>, ApiError> {
    require_admin(&headers, &state)?;
    let (config, paths) = scoped(&state, &query.persona)?;
    let id = query.id.trim().to_string();
    if id.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "missing skill id"));
    }
    let payload = tokio::task::spawn_blocking(move || {
        // 先走 discover 确认这个技能在当前视角下真的可见——直接读目录会
        // 绕过白名单，用户能从面板看到模型根本碰不到的正文。
        let entries = miyu_core::skills::discover(&config, &paths)?;
        let Some(entry) = entries.iter().find(|entry| entry.metadata.name == id) else {
            return Ok::<Value, anyhow::Error>(json!({
                "id": id,
                "missing": true,
            }));
        };
        let body = entry
            .directory
            .as_ref()
            .map(|dir| std::fs::read_to_string(dir.join("SKILL.md")))
            .transpose()?
            .unwrap_or_default();
        Ok(json!({
            "id": entry.metadata.name,
            "missing": false,
            "source": entry.source.as_str(),
            "body": body,
        }))
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(|error: anyhow::Error| ApiError::internal(safe_error_message(&error)))?;
    Ok(Json(payload))
}

/// 禁用用的是 `.disabled` 标记文件——与 `miyu skills disable` 同一套约定，
/// 不给面板另发明一套状态。
fn toggle_marker(
    state: &DaemonState,
    persona: &str,
    id: &str,
    disable: bool,
) -> std::result::Result<Value, ApiError> {
    let (config, paths) = scoped(state, persona)?;
    let entries = miyu_core::skills::discover(&config, &paths)
        .map_err(|error| ApiError::internal(safe_error_message(&error)))?;
    let Some(entry) = entries.iter().find(|entry| entry.metadata.name == id) else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "skill not found"));
    };
    // 内置技能住在资源树里，别去写它——那是安装目录，改坏了升级要重装。
    if entry.source == SkillSource::BuiltIn {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "built-in skills cannot be toggled from the dashboard",
        ));
    }
    let Some(directory) = entry.directory.as_ref() else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "skill has no directory",
        ));
    };
    let marker = directory.join(".disabled");
    if disable {
        std::fs::write(&marker, b"disabled from the WebUI dashboard\n")
            .map_err(|error| ApiError::internal(safe_error_message(&error)))?;
    } else if marker.exists() {
        std::fs::remove_file(&marker)
            .map_err(|error| ApiError::internal(safe_error_message(&error)))?;
    }
    Ok(json!({ "id": id, "disabled": disable }))
}

/// `POST /api/dash/skills/disable`
pub(in crate::web) async fn dash_skills_disable(
    State(state): State<DaemonState>,
    headers: HeaderMap,
    Json(body): Json<SkillIdBody>,
) -> std::result::Result<Json<Value>, ApiError> {
    require_admin_mutation(&headers, &state)?;
    Ok(Json(toggle_marker(&state, &body.persona, &body.id, true)?))
}

/// `POST /api/dash/skills/enable`
pub(in crate::web) async fn dash_skills_enable(
    State(state): State<DaemonState>,
    headers: HeaderMap,
    Json(body): Json<SkillIdBody>,
) -> std::result::Result<Json<Value>, ApiError> {
    require_admin_mutation(&headers, &state)?;
    Ok(Json(toggle_marker(&state, &body.persona, &body.id, false)?))
}
