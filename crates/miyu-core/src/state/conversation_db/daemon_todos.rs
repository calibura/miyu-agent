//! daemon 级轻量待办。
//!
//! 会话内的 todo 工具住 `session_values`,跟会话走;这份是 daemon 自己的
//! 台面账:WebUI 目标面板上随手记的几件事,不分会话、不分会话归属,删会话
//! 也不级联。轻量就轻量到底:没有排序、没有标签,只有内容/优先级/状态。

use crate::state::conversation_db::*;

/// 一条待办。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DaemonTodo {
    pub id: i64,
    pub content: String,
    pub priority: String,
    pub status: String,
    pub created_at: String,
    pub completed_at: Option<String>,
}

const TODO_COLUMNS: &str = "id, content, priority, status, created_at, completed_at";

fn todo_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DaemonTodo> {
    Ok(DaemonTodo {
        id: row.get(0)?,
        content: row.get(1)?,
        priority: row.get(2)?,
        status: row.get(3)?,
        created_at: row.get(4)?,
        completed_at: row.get(5)?,
    })
}

/// 优先级限定三档。自由文本的话很快就会混进模型编的词,列表没法按档排序。
fn priority_valid(priority: &str) -> bool {
    matches!(priority, "high" | "medium" | "low")
}

impl ConversationDb {
    /// 全部待办,未完成的在前、新记的在前。
    pub fn daemon_todos(&self) -> Result<Vec<DaemonTodo>> {
        let conn = self.conn.lock().unwrap();
        let mut statement = conn.prepare(&format!(
            "SELECT {TODO_COLUMNS} FROM daemon_todos
             ORDER BY (status = 'open') DESC, id DESC"
        ))?;
        let rows = statement.query_map([], todo_from_row)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 记一条。内容不能空,优先级三档,其余不设限。
    pub fn create_daemon_todo(&self, content: &str, priority: &str) -> Result<DaemonTodo> {
        let content = content.trim();
        if content.is_empty() {
            bail!("todo content must be a non-empty string");
        }
        if !priority_valid(priority) {
            bail!("todo priority must be one of: high, medium, low");
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO daemon_todos (content, priority, status, created_at)
             VALUES (?1, ?2, 'open', ?3)",
            params![content, priority, Utc::now().to_rfc3339()],
        )?;
        let id = conn.last_insert_rowid();
        let mut statement = conn.prepare(&format!(
            "SELECT {TODO_COLUMNS} FROM daemon_todos WHERE id = ?1"
        ))?;
        Ok(statement.query_row(params![id], todo_from_row)?)
    }

    /// 改状态或优先级。勾完成时盖 completed_at,退回时抹掉——completed_at
    /// 有值而 status 是 open 的行,是展示层的坏数据。
    pub fn update_daemon_todo(
        &self,
        id: i64,
        status: Option<&str>,
        priority: Option<&str>,
    ) -> Result<DaemonTodo> {
        if let Some(priority) = priority {
            if !priority_valid(priority) {
                bail!("todo priority must be one of: high, medium, low");
            }
        }
        if let Some(status) = status {
            if !matches!(status, "open" | "done") {
                bail!("todo status must be one of: open, done");
            }
        }
        if status.is_none() && priority.is_none() {
            bail!("todo update requires status and/or priority");
        }
        let conn = self.conn.lock().unwrap();
        let completed_at: Option<&str> = match status {
            Some("done") => Some(&Utc::now().to_rfc3339()),
            Some(_) => Some(""),
            None => None,
        };
        conn.execute(
            "UPDATE daemon_todos SET
                 status = COALESCE(?2, status),
                 priority = COALESCE(?3, priority),
                 completed_at = CASE
                     WHEN ?2 = 'done' THEN ?4
                     WHEN ?2 = 'open' THEN NULL
                     ELSE completed_at END
             WHERE id = ?1",
            params![id, status, priority, completed_at.unwrap_or_default()],
        )?;
        let mut statement = conn.prepare(&format!(
            "SELECT {TODO_COLUMNS} FROM daemon_todos WHERE id = ?1"
        ))?;
        Ok(statement
            .query_row(params![id], todo_from_row)
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("todo {id} not found"))?)
    }

    /// 删一条。id 不存在当成功:删的目标就是「这条没了」。
    pub fn delete_daemon_todo(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM daemon_todos WHERE id = ?1", params![id])?;
        Ok(())
    }
}
