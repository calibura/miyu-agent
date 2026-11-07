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

use crate::runtime::{all_goal_overview, GoalOverviewRow};
use crate::web::*;
use miyu_core::state::DaemonTodo;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths(root: &std::path::Path) -> MiyuPaths {
        MiyuPaths {
            root_dir: root.to_path_buf(),
            config_dir: root.join("config"),
            config_file: root.join("config/config.jsonc"),
            skills_dir: root.join("config/skills"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
            state_dir: root.join("state"),
            pictures_dir: root.join("pictures"),
            fish_hook_file: root.join("fish"),
            bash_hook_file: root.join("bash"),
            zsh_hook_file: root.join("zsh"),
            scripts_dir: root.join("scripts"),
            system_scripts_dir: root.join("system-scripts"),
        }
    }

    /// 跨库总览(管理员视角):管理员库和成员库里的目标都得出,带归属、按
    /// updated_at 倒序。这是 TUI `/goals` 与 WebUI `/api/goals` 共用的读法,
    /// 两个界面靠它不各写一份扫库。禁用的账号跳过——他的目标不再出现在
    /// 总览里,和 WebUI 原来的口径一致。
    #[test]
    fn goal_overview_covers_admin_and_member_stores() {
        let temp = tempfile::tempdir().unwrap();
        let state = DaemonState::for_test(test_paths(temp.path()), 8300).unwrap();
        let persona = state.manager.lock().unwrap().config.active_persona_scope();

        // 管理员库里的目标。
        let admin_session = state
            .state_store
            .create_session_for_owner(
                &persona,
                "写代码",
                miyu_core::state::USER_SESSION_KIND,
                None,
                "",
            )
            .unwrap()
            .session_id;
        state
            .state_store
            .create_goal(&admin_session, "把测试跑绿", None)
            .unwrap();

        // 成员账号 + 成员库里的目标。成员库由 StoreRegistry 第一次 for_owner 开。
        // 先建管理员账号:第一个账号会被无条件提成管理员,没有它成员就成了管理员。
        state
            .state_store
            .create_account(
                "admin",
                "管理员",
                "pwd-123456",
                miyu_core::state::ROLE_ADMIN,
            )
            .unwrap();
        let account = state
            .state_store
            .create_account(
                "xiaoming",
                "小明",
                "pwd-123456",
                miyu_core::state::ROLE_MEMBER,
            )
            .unwrap();
        assert!(!account.is_admin(), "第二个账号才是成员");
        let member_store = state.stores.for_owner(&account.id).unwrap();
        let member_session = member_store
            .create_session_for_owner(
                &persona,
                "成员的会话",
                miyu_core::state::USER_SESSION_KIND,
                None,
                &account.id,
            )
            .unwrap()
            .session_id;
        member_store
            .create_goal(&member_session, "整理相册", None)
            .unwrap();

        let rows = all_goal_overview(&state.stores);
        assert_eq!(rows.len(), 2, "管理员和成员的目标都要在");
        // 后建的目标 updated_at 更大,排前面。
        assert_eq!(rows[0].record.objective, "整理相册");
        assert_eq!(rows[0].owner, account.id);
        assert_eq!(rows[0].session_name, "成员的会话");
        assert_eq!(rows[1].record.objective, "把测试跑绿");
        assert_eq!(rows[1].owner, "", "管理员库的归属是空串");

        // 禁用后他的目标不再出现。
        state
            .state_store
            .set_account_disabled(&account.id, true)
            .unwrap();
        let rows = all_goal_overview(&state.stores);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].record.objective, "把测试跑绿");
    }
}

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
    // 管理员走 `all_goal_overview`(管理员库 + 所有成员库,TUI 的 `/goals`
    // 走同一条);成员只看自己名下的,扫库收窄到自己那一个库。
    let goals = if identity.admin {
        all_goal_overview(&state.stores)
    } else {
        state
            .stores
            .for_identity(&identity)
            .map_err(ApiError::internal)?
            .goals_with_session_names()
            .map(|rows| {
                rows.into_iter()
                    .map(|(record, session_name)| GoalOverviewRow {
                        owner: identity.owner_key().to_string(),
                        record,
                        session_name,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let items = goals
        .iter()
        .map(|row| goal_json(&row.record, &row.session_name))
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({ "goals": items })))
}

/// 与 `goal_json` 同形,补上归属。TUI 的总览要看见「这是谁的目标」;WebUI
/// 侧栏只给自己的帐号,没要这个字段。
///
/// 原先还有一个不带 `owner` 的 `goals_overview_json`，注释说「IPC 与 HTTP
/// 共用」，但两处都只调这个带归属的版本，那个函数从加进来就没被调用过。
/// 2026-11-07 删掉，dead_code 警告随之消失。
pub(in crate::web) fn goals_overview_with_owner_json(
    rows: &[GoalOverviewRow],
) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|row| {
            let mut value = goal_json(&row.record, &row.session_name);
            value
                .as_object_mut()
                .map(|object| object.insert("owner".into(), serde_json::json!(row.owner)));
            value
        })
        .collect()
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

/* ── daemon 级轻量待办 ───────────────────────────────── */

fn daemon_todo_json(todo: &DaemonTodo) -> serde_json::Value {
    serde_json::json!({
        "id": todo.id,
        "content": todo.content,
        "priority": todo.priority,
        "status": todo.status,
        "created_at": todo.created_at,
        "completed_at": todo.completed_at,
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::web) struct CreateTodoRequest {
    #[serde(default)]
    content: String,
    #[serde(default)]
    priority: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::web) struct UpdateTodoRequest {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    priority: Option<String>,
}

/// daemon 台面账 CRUD。管理员读写自己的库;成员库各管各的台面,同理。
pub(in crate::web) async fn daemon_todos_http(
    State(state): State<DaemonState>,
    headers: HeaderMap,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    let identity = require_identity(&headers, &state)?;
    let store = state
        .stores
        .for_identity(&identity)
        .map_err(ApiError::internal)?;
    let todos = store
        .daemon_todos()
        .map_err(|error| ApiError::internal(safe_error_message(&error)))?;
    let items = todos.iter().map(daemon_todo_json).collect::<Vec<_>>();
    Ok(Json(serde_json::json!({ "todos": items })))
}

pub(in crate::web) async fn daemon_todo_create_http(
    State(state): State<DaemonState>,
    headers: HeaderMap,
    Json(request): Json<CreateTodoRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    require_mutation(&headers, &state)?;
    let identity = require_identity(&headers, &state)?;
    let store = state
        .stores
        .for_identity(&identity)
        .map_err(ApiError::internal)?;
    let priority = if request.priority.trim().is_empty() {
        "medium"
    } else {
        request.priority.trim()
    };
    let todo = store
        .create_daemon_todo(&request.content, priority)
        .map_err(|error| ApiError::new(StatusCode::BAD_REQUEST, safe_error_message(&error)))?;
    Ok(Json(
        serde_json::json!({ "ok": true, "todo": daemon_todo_json(&todo) }),
    ))
}

pub(in crate::web) async fn daemon_todo_update_http(
    State(state): State<DaemonState>,
    headers: HeaderMap,
    Path(todo_id): Path<i64>,
    Json(request): Json<UpdateTodoRequest>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    require_mutation(&headers, &state)?;
    let identity = require_identity(&headers, &state)?;
    let store = state
        .stores
        .for_identity(&identity)
        .map_err(ApiError::internal)?;
    let result = store.update_daemon_todo(
        todo_id,
        request.status.as_deref(),
        request.priority.as_deref(),
    );
    match result {
        Ok(todo) => Ok(Json(
            serde_json::json!({ "ok": true, "todo": daemon_todo_json(&todo) }),
        )),
        Err(error) => Err(ApiError::new(
            StatusCode::CONFLICT,
            safe_error_message(&error),
        )),
    }
}

pub(in crate::web) async fn daemon_todo_delete_http(
    State(state): State<DaemonState>,
    headers: HeaderMap,
    Path(todo_id): Path<i64>,
) -> std::result::Result<Json<serde_json::Value>, ApiError> {
    require_mutation(&headers, &state)?;
    let identity = require_identity(&headers, &state)?;
    let store = state
        .stores
        .for_identity(&identity)
        .map_err(ApiError::internal)?;
    store
        .delete_daemon_todo(todo_id)
        .map_err(|error| ApiError::internal(safe_error_message(&error)))?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
