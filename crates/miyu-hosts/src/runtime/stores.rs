//! 会话库按人分(09-10 阶段 8):管理员一份(根布局 `home/<admin>/conversation.db`,
//! 终端/语音/遗留都在里面),每个成员自己一份(`home/<用户>/conversation.db`)。
//!
//! Web 层拿到登录身份就问「他的库」;actor 与事件流手里只有 session id,就
//! 问「这个会话在谁的库里」——先查管理员的,再查开过的成员库,最后按账号表
//! 把没开过的库开一遍。会话 id 全局唯一(时间戳 + 随机),查到即定,结果缓存。
//! 账号表、邀请码、用量账本仍在管理员那份/state 目录,不分家。

use super::*;
use anyhow::{Context, Result};
use miyu_base::paths::MiyuPaths;
use miyu_core::state::StateStore;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(crate) struct StoreRegistry {
    admin: StateStore,
    paths: MiyuPaths,
    /// 账号 id → 成员库
    members: Arc<Mutex<HashMap<String, StateStore>>>,
    /// 会话 id → 账号 id(空串 = 管理员)
    owners: Arc<Mutex<HashMap<String, String>>>,
}

impl StoreRegistry {
    pub fn new(admin: StateStore, paths: MiyuPaths) -> Self {
        Self {
            admin,
            paths,
            members: Arc::new(Mutex::new(HashMap::new())),
            owners: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 某个账号的库;空串 = 管理员。成员库第一次用时打开(建目录、跑迁移)。
    pub(crate) fn for_owner(&self, owner: &str) -> Result<StateStore> {
        if owner.is_empty() {
            return Ok(self.admin.clone());
        }
        if let Some(store) = self.members.lock().unwrap().get(owner) {
            return Ok(store.clone());
        }
        let account = self
            .admin
            .account_by_id(owner)?
            .with_context(|| format!("unknown account {owner}"))?;
        let store = StateStore::open_member(&self.paths, &account.username)?;
        let _ = store.recover_stale_turns();
        // 成员库第一次打开就在本进程:里面还标着 running/waiting 的子代理任务只能是
        // 上个进程留下的(本进程跑过的子会话早把库打开了),一律标 interrupted。
        let _ = store.mark_subagent_tasks_interrupted();
        let mut members = self.members.lock().unwrap();
        Ok(members.entry(owner.to_string()).or_insert(store).clone())
    }

    pub(crate) fn for_identity(&self, identity: &WebIdentity) -> Result<StateStore> {
        self.for_owner(identity.owner_key())
    }

    /// 会话在谁的库里:找不到就当管理员的(旧路径的 404 语义不变)。
    pub(crate) fn owner_of_session(&self, session_id: &str) -> Option<String> {
        if let Some(owner) = self.owners.lock().unwrap().get(session_id) {
            return Some(owner.clone());
        }
        let owner = self.locate_session(session_id)?;
        let mut owners = self.owners.lock().unwrap();
        if owners.len() > 8_192 {
            owners.clear();
        }
        owners.insert(session_id.to_string(), owner.clone());
        Some(owner)
    }

    pub(crate) fn for_session(&self, session_id: &str) -> StateStore {
        match self.owner_of_session(session_id) {
            Some(owner) => self
                .for_owner(&owner)
                .unwrap_or_else(|_| self.admin.clone()),
            None => self.admin.clone(),
        }
    }

    fn locate_session(&self, session_id: &str) -> Option<String> {
        if matches!(self.admin.session_record(session_id), Ok(Some(_))) {
            return Some(String::new());
        }
        let opened: Vec<(String, StateStore)> = self
            .members
            .lock()
            .unwrap()
            .iter()
            .map(|(owner, store)| (owner.clone(), store.clone()))
            .collect();
        for (owner, store) in &opened {
            if matches!(store.session_record(session_id), Ok(Some(_))) {
                return Some(owner.clone());
            }
        }
        // 没开过的成员库:按账号表逐个开(只在冷启动后第一次碰到别人的会话时)
        let accounts = self.admin.list_accounts().ok()?;
        for account in accounts {
            if account.is_admin() || opened.iter().any(|(owner, _)| owner == &account.id) {
                continue;
            }
            let Ok(store) = self.for_owner(&account.id) else {
                continue;
            };
            if matches!(store.session_record(session_id), Ok(Some(_))) {
                return Some(account.id);
            }
        }
        None
    }

    /// 新建会话后登记归属,免得马上又扫一遍。
    pub(crate) fn note_session_owner(&self, session_id: &str, owner: &str) {
        self.owners
            .lock()
            .unwrap()
            .insert(session_id.to_string(), owner.to_string());
    }

    /// 账号表(管理员库里):跨库扫目标要用。
    pub(crate) fn list_accounts(&self) -> Vec<miyu_core::state::Account> {
        self.admin.list_accounts().unwrap_or_default()
    }
}

/// 一行跨库目标总览:目标本身 + 会话名 + 归属(空串 = 管理员,其余是账号 id)。
pub(crate) struct GoalOverviewRow {
    pub owner: String,
    pub record: miyu_core::state::GoalRecord,
    pub session_name: String,
}

/// 跨库扫所有目标:管理员库一批,再扫账号表上每个成员的库(没开过的也开——
/// 与 WebUI `/api/goals` 同一份口径,漏一个成员的目标比多开一个库文件贵)。
/// 按 updated_at 倒序混排:总览要的是「现在都在忙什么」,顺序读的是时间。
///
/// 有 TUI(`/goals`)与 WebUI(`/api/goals`)两个客户:同一份读法,两边各拼
/// 各的展示。谁名下的会话在谁的库,这里按 StoreRegistry 自己的缓存与账号
/// 表定,读库口径与 `for_owner` 一致。
pub(crate) fn all_goal_overview(registry: &StoreRegistry) -> Vec<GoalOverviewRow> {
    let mut out = Vec::new();
    let push = |owner: &str, store: StateStore, out: &mut Vec<GoalOverviewRow>| match store
        .goals_with_session_names()
    {
        Ok(rows) => {
            out.extend(
                rows.into_iter()
                    .map(|(record, session_name)| GoalOverviewRow {
                        owner: owner.to_string(),
                        record,
                        session_name,
                    }),
            );
        }
        Err(error) => {
            tracing::warn!(owner = %owner, error = %error, "goal overview: a store could not be read")
        }
    };
    if let Ok(store) = registry.for_owner("") {
        push("", store, &mut out);
    }
    let accounts = registry.list_accounts();
    for account in accounts {
        if account.disabled || account.is_admin() {
            continue;
        }
        if let Ok(store) = registry.for_owner(&account.id) {
            push(&account.id, store, &mut out);
        }
    }
    out.sort_by(|a, b| b.record.updated_at.cmp(&a.record.updated_at));
    out
}
