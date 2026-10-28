//! WebUI 的目标总览与管理面。
//!
//! `/api/goals` 跨会话列出所有目标;`/api/goals/{session_id}/{action}` 走
//! `ConversationDb` 的同一套 CAS 状态机(pause/resume/complete),与 `/goal`
//! 命令、goal 工具共用底层转换——阶段合法性与撞版本拒绝只有一份。与
//! `/api/goal`(斜杠命令回执)不同,这里返回结构化 JSON,给「目标」面板的
//! 列表与按钮用,且操作显式带 CAS 凭证(goal_id + revision)。
//!
//! WebUI 发起的 pause/resume/complete 都落审计日志:谁在什么时候对哪个会话
//! 的目标做了什么,只进 tracing,不进库——目标本身的全过程在对话记录里有。

use crate::web::*;

/// 一个目标的出口形态:目标字段 + 会话标识。
fn goal_json(record: &miyu_core::state::GoalRecord, session_name: &str) -> serde_json::Value {
    serde_json::json!({
        "session_id": record.session_id,
        "session_name": session_name,
        "goal_id": record.goal_id,
        "revision": record.revision,
        "objective": record.objective,
        "phase": record.phase.as_str(),
        "rounds_started": record.rounds_started,
        "max_rounds": record.max_rounds,
        "blocked_code": record.blocked_code,
        "blocked_message": record.blocked_message,
        "created_at": record.created_at,
        "updated_at": record.updated_at,
    })
}

/// 跨会话目标总览。管理员看全家的;成员只看自己名下的(自己的库)。
pub(in crate::web) async fn goals_overview_http(
    State(state): State<DaemonState>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let identity = require_identity(&headers, &state)?;
    let mut goals = Vec::new();
    if identity.admin {
        // 管理员库一批,再扫已打开的成员库。未打开的成员库也开:目标列表
        // 要全,漏一个成员的目标比多开一个库文件贵。
        if let Ok(store) = state.stores.for_owner("") {
            push_store_goals(store, &mut goals);
        }
        let accounts = state.state_store.list_accounts().unwrap_or_default();
        for account in accounts {
            if account.disabled {
                continue;
            }
            if let Ok(store) = state.stores.for_owner(&account.id) {
                push_store_goals(store, &mut goals);
            }
        }
    } else {
        let store = state
            .stores
            .for_identity(&identity)
            .map_err(ApiError::internal)?;
        push_store_goals(store, &mut goals);
    }
    goals.sort_by(|a, b| b.0.updated_at.cmp(&a.0.updated_at));
    let items = goals
        .iter()
        .map(|(record, name)| goal_json(record, name))
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({ "goals": items })))
}

fn push_store_goals(
    store: miyu_core::state::StateStore,
    out: &mut Vec<(miyu_core::state::GoalRecord, String)>,
) {
    match store.goals_with_session_names() {
        Ok(rows) => out.extend(rows),
        Err(error) => tracing::warn!(error = %error, "goal overview: a store could not be read"),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::web) struct GoalActionRequest {
    #[serde(default)]
    goal_id: String,
    #[serde(default)]
    revision: i64,
}

/// WebUI 发起的目标操作:pause / resume / complete。
///
/// 走 `/goal` 命令的同一份状态机;与斜杠命令不同的是这里显式带 CAS 凭证
/// (goal_id + revision),撞版本把当前凭证原样带回,前端重拉一次列表就好。
/// resume 之后踢驱动器一脚(与 `/goal resume` 同语义),pause 之后掐掉在飞的
/// 续轮(与 `/goal pause` 同语义)。
pub(in crate::web) async fn goal_action_http(
    State(state): State<DaemonState>,
    headers: HeaderMap,
    Path((session_id, action)): Path<(String, String)>,
    Json(request): Json<GoalActionRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    require_mutation(&headers, &state)?;
    let identity = require_identity(&headers, &state)?;
    require_local_web_session(&state, &headers, &session_id)?;
    let action = match action.as_str() {
        "pause" | "resume" | "complete" => action,
        other => {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("unknown goal action: {other}"),
            ))
        }
    };
    if request.goal_id.trim().is_empty() || request.revision < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "goal_id and revision are required",
        ));
    }
    // 目标操作在会话所属者的库里执行(goals 表对 sessions 有外键)。
    let store = state.stores.for_session(&session_id).pinned(&session_id);
    let result = match action.as_str() {
        "pause" => store.pause_goal(&session_id, &request.goal_id, request.revision),
        "resume" => store.resume_goal(&session_id, &request.goal_id, request.revision),
        _ => store.complete_goal(&session_id, &request.goal_id, request.revision),
    };
    match result {
        Ok(record) => {
            tracing::info!(
                session_id = %session_id,
                goal_id = %record.goal_id,
                action = %action,
                actor = %identity.display_name,
                actor_id = %identity.account_id,
                revision = record.revision,
                "goal action from web ui"
            );
            match action.as_str() {
                // 与 `/goal pause` 同语义:立刻停住正在跑的续轮,不然用户看到
                // 的是「还在跑」。complete 不掐——收尾轮让它自然跑完。
                "pause" => cancel_goal_rounds_for_session(&state, &session_id),
                // 与 `/goal resume` 同语义:人重新授权了,该立刻接着跑。
                "resume" => miyu_engine::tools::goal::set_armed(&session_id, true),
                _ => miyu_engine::tools::goal::set_armed(&session_id, false),
            }
            if action == "resume" {
                nudge_goal_driver(&state, &session_id);
            }
            Ok(Json(
                serde_json::json!({ "ok": true, "goal": goal_json(&record, "") }),
            ))
        }
        Err(error) => {
            tracing::info!(
                session_id = %session_id,
                action = %action,
                actor = %identity.display_name,
                error = %error,
                "goal action from web ui was denied"
            );
            Err(ApiError::new(
                StatusCode::CONFLICT,
                safe_error_message(&error),
            ))
        }
    }
}
