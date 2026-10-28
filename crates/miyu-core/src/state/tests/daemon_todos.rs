//! daemon 级轻量待办的读写。
//!
//! 这份清单是给人随手记的台面账,断言围绕两条:入口校验不放过垃圾
//! (空内容、越权优先级/状态),以及 completed_at 与 status 永不自相矛盾。

use super::shared::*;
use crate::state::*;

#[test]
fn daemon_todo_crud_roundtrip() {
    let (_t, store) = test_store();
    assert!(store.daemon_todos().unwrap().is_empty());

    let todo = store.create_daemon_todo("把 crates 拆完", "high").unwrap();
    assert_eq!(todo.content, "把 crates 拆完");
    assert_eq!(todo.priority, "high");
    assert_eq!(todo.status, "open");
    assert!(todo.completed_at.is_none());

    // 勾完成:盖 completed_at,排到未完成之后。
    let done = store
        .update_daemon_todo(todo.id, Some("done"), None)
        .unwrap();
    assert_eq!(done.status, "done");
    assert!(done.completed_at.is_some());
    let listed = store.daemon_todos().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, "done");

    // 退回:completed_at 抹掉,不留「open 却带着完成时间」的坏数据。
    let reopened = store
        .update_daemon_todo(todo.id, Some("open"), None)
        .unwrap();
    assert_eq!(reopened.status, "open");
    assert!(reopened.completed_at.is_none());

    // 删除:不存在当成功。
    store.delete_daemon_todo(todo.id).unwrap();
    store.delete_daemon_todo(999_999).unwrap();
    assert!(store.daemon_todos().unwrap().is_empty());
}

#[test]
fn daemon_todo_rejects_garbage_input() {
    let (_t, store) = test_store();

    assert!(store.create_daemon_todo("   ", "high").is_err());
    assert!(store.create_daemon_todo("内容", "urgent").is_err());
    assert!(store.create_daemon_todo("内容", "medium").is_ok());

    let todo = store.daemon_todos().unwrap().remove(0);
    assert!(store
        .update_daemon_todo(todo.id, Some("archived"), None)
        .is_err());
    assert!(store
        .update_daemon_todo(todo.id, None, Some("critical"))
        .is_err());
    assert!(store.update_daemon_todo(todo.id, None, None).is_err());
    assert!(store
        .update_daemon_todo(999_999, Some("done"), None)
        .is_err());
}
