//! 撤回、上传、禁言等通知事件。

use super::shared::*;
use crate::platforms::onebot::*;

/// 群禁言缓存是进程全局的 `OnceLock<Mutex<GroupMuteCache>>`(`caches.rs`),几条测试各自
/// 往里写再读;并行跑时互相踩(09-16 门禁两次抖动都在这里)。碰它的测试拿这把锁串行,
/// 上一条 panic 留下的中毒锁照拿。
static MUTE_CACHE_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serialize_mute_cache_tests() -> std::sync::MutexGuard<'static, ()> {
    MUTE_CACHE_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 戳一戳:SnowLuma 送 `notice_type=notify, sub_type=poke`,`user_id` 是动手的
/// 人、`target_id` 是被戳的人。改写成一条文本消息后必须仍带上原来那个发信人,
/// 否则限流、白名单、好感度都会记到错的人头上。
#[test]
fn a_group_poke_becomes_a_message_from_the_poker() {
    let event = json!({
        "post_type": "notice",
        "notice_type": "notify",
        "sub_type": "poke",
        "time": 1793026000,
        "self_id": 10000,
        "group_id": 130515,
        "user_id": 29313,
        "target_id": 10000,
        "suffix": "拍了拍我的头",
        "sender": { "nickname": "戳人的那个" }
    });
    assert!(is_poke_notice(&event), "戳一戳应被认出来");
    let message = poke_as_message(&event).expect("poke should convert to a message");
    assert_eq!(message["post_type"], "message");
    assert_eq!(message["message_type"], "group");
    assert_eq!(message["group_id"], 130515);
    assert_eq!(message["self_id"], 10000);
    assert_eq!(message["user_id"], 29313, "发信人必须是戳人的那个");
    assert_eq!(message["sender"]["nickname"], "戳人的那个");
    let text = message["raw_message"].as_str().unwrap_or_default();
    assert!(text.contains("拍了拍我的头"), "suffix 要带进文本: {text}");
    // 序号必须成对出现:下游按平台序号排到达顺序、定位引用与撤回,缺了整条
    // 会被当成"悄悄消失"的消息丢掉(实测:戳了没反应就是这么来的)。
    assert!(message.get("message_id").is_some(), "合成消息必须有 message_id");
    assert!(
        message.get("real_seq").and_then(|v| v.as_str()).is_some(),
        "合成消息必须带 real_seq(字符串),否则下游丢事件"
    );
    // 群聊的唤醒条件只认「@我 / 引用我 / 唤醒词开头」,戳一戳三者都不满足。
    // 合成消息必须自带一个 at 机器人自己的段,否则整条会被静默丢掉。
    let segments = message["message"].as_array().expect("message 应是段数组");
    let ats_self = segments.iter().any(|seg| {
        seg.get("type").and_then(|v| v.as_str()) == Some("at")
            && seg
                .get("data")
                .and_then(|d| d.get("qq"))
                .and_then(|q| q.as_str())
                .is_some_and(|qq| qq == "10000")
    });
    assert!(ats_self, "合成消息必须 @ 机器人自己,否则群聊唤醒不了");
}

/// 私聊戳没有 `group_id`,得落成私聊消息,而且 target 缺省时按动手的人算。
#[test]
fn a_private_poke_without_group_becomes_a_private_message() {
    let event = json!({
        "post_type": "notice",
        "notice_type": "notify",
        "sub_type": "poke",
        "time": 1793026001,
        "self_id": 10000,
        "user_id": 29313,
        "target_id": 10000
    });
    let message = poke_as_message(&event).expect("private poke should convert");
    assert_eq!(message["message_type"], "private");
    assert!(message.get("group_id").is_none(), "私聊不该带 group_id");
    let text = message["raw_message"].as_str().unwrap_or_default();
    assert!(text.contains("戳"), "没有 suffix 时也要说清这是一次戳: {text}");
}

/// SnowLuma 的**私聊戳**字段语义与群戳不同:`user_id` 是会话对端(机器人
/// 自己),动手的人在 `sender_id` 里。取错了会把私聊戳当成机器人自己的消息丢掉。
#[test]
fn a_private_poke_takes_the_poker_from_sender_id() {
    let event = json!({
        "post_type": "notice",
        "notice_type": "notify",
        "sub_type": "poke",
        "time": 1793026006,
        "self_id": 1667519583i64,
        "user_id": 1667519583i64,
        "sender_id": 2290154072i64,
        "target_id": 1667519583i64,
        "suffix": ""
    });
    assert_eq!(poke_poker_id(&event), Some(2290154072i64), "动手的人是 sender_id");
    let message = poke_as_message(&event).expect("private poke should convert");
    assert_eq!(message["message_type"], "private");
    assert_eq!(
        message["user_id"], 2290154072i64,
        "发信人必须是戳人的那个,不是会话对端"
    );
}

/// `notice_type=poke` 是少数实现的变体,也要认。
#[test]
fn the_bare_poke_notice_type_is_also_accepted() {
    let event = json!({
        "post_type": "notice",
        "notice_type": "poke",
        "time": 1793026002,
        "self_id": 10000,
        "user_id": 29313,
        "target_id": 10000
    });
    assert!(is_poke_notice(&event));
}

/// 账号上下线:被踢下线仍然是 `bot_offline`,SnowLuma 的常规上下线是 `bot_status`。
#[test]
fn bot_status_and_bot_offline_are_both_recognised() {
    for notice_type in ["bot_status", "bot_offline"] {
        let event = json!({
            "post_type": "notice",
            "notice_type": notice_type,
            "sub_type": "offline",
            "self_id": 10000,
            "time": 1793026003
        });
        assert!(is_bot_status_notice(&event), "{notice_type} 应被认出来");
    }
    let message = json!({
        "post_type": "message",
        "message_type": "group",
        "self_id": 10000,
        "user_id": 29313,
        "group_id": 130515
    });
    assert!(!is_bot_status_notice(&message), "普通消息不该被当成上下线");
}

/// 群成员增加:自己进群时 `mentioned_bot` 为真,用来触发群名刷新。
#[test]
fn a_group_increase_marks_self_joins() {
    let self_join = json!({
        "post_type": "notice",
        "notice_type": "group_increase",
        "sub_type": "approve",
        "time": 1793026004,
        "self_id": 10000,
        "group_id": 130515,
        "user_id": 10000,
        "operator_id": 29313
    });
    assert!(is_group_increase_notice(&self_join));
    let inbound = group_increase_event(&self_join).expect("group increase should parse");
    assert!(inbound.mentioned_bot, "自己进群应标记为与自己相关");
    assert_eq!(inbound.operator_id.as_deref(), Some("29313"));

    let other_join = json!({
        "post_type": "notice",
        "notice_type": "group_increase",
        "time": 1793026005,
        "self_id": 10000,
        "group_id": 130515,
        "user_id": 29313
    });
    let inbound = group_increase_event(&other_join).expect("other joins should parse too");
    assert!(!inbound.mentioned_bot, "别人进群与自己无关");
}

#[test]
fn group_upload_notice_becomes_a_file_history_event() {
    let event = json!({
        "post_type": "notice",
        "notice_type": "group_upload",
        "time": 1786691192,
        "self_id": 10000,
        "group_id": 130515,
        "user_id": 29313,
        "file": {
            "id": "/8b25e30e-8ee2-4223-9e30-fd45ee24c797",
            "name": "配置.txt",
            "size": 11035,
            "busid": 102
        }
    });
    let inbound = group_upload_event(&event).expect("group upload notice should parse");
    assert_eq!(inbound.kind, PlatformInboundEventKind::GroupFileUpload);
    assert_eq!(inbound.sender_id, "29313");
    assert_eq!(inbound.timestamp, 1786691192);
    assert_eq!(
        inbound.message_id,
        "group_file_8b25e30e-8ee2-4223-9e30-fd45ee24c797"
    );
    assert_eq!(inbound.media.len(), 1);
    assert_eq!(inbound.media[0].kind, PlatformMediaKind::File);
    assert_eq!(
        inbound.media[0].id.as_deref(),
        Some("/8b25e30e-8ee2-4223-9e30-fd45ee24c797")
    );
    assert_eq!(inbound.media[0].name.as_deref(), Some("配置.txt"));
    assert!(inbound.ingress_order.is_some());
}

#[test]
fn recall_notices_become_structured_inbound_events() {
    let event = json!({
        "post_type": "notice",
        "notice_type": "group_recall",
        "self_id": 10000,
        "group_id": 42,
        "user_id": 7,
        "operator_id": 8,
        "message_id": 99,
        "time": 123,
    });
    assert!(is_message_recall(&event));
    let recalled = recall_event(Target::Group { group_id: 42 }, &event, 7);
    assert_eq!(recalled.kind, PlatformInboundEventKind::MessageRecall);
    assert_eq!(recalled.conversation.account_id, "10000");
    assert_eq!(recalled.conversation.conversation_id, "42");
    assert_eq!(recalled.message_id, "99");
    assert_eq!(recalled.sender_id, "7");
    assert_eq!(recalled.operator_id.as_deref(), Some("8"));
    assert_eq!(recalled.timestamp, 123);

    assert!(!is_message_recall(&json!({
        "post_type": "notice",
        "notice_type": "group_increase"
    })));
}

#[test]
fn group_mute_cache_expires_and_isolates_bot_accounts() {
    let start = Instant::now();
    let mut cache = GroupMuteCache::default();
    cache.insert(
        (10_001, 42),
        BotSendAvailability::Muted,
        Duration::from_secs(5),
        start,
    );
    cache.insert(
        (10_002, 42),
        BotSendAvailability::Available,
        Duration::from_secs(5),
        start,
    );
    assert_eq!(
        cache.get((10_001, 42), start),
        Some(BotSendAvailability::Muted)
    );
    assert_eq!(
        cache.get((10_002, 42), start),
        Some(BotSendAvailability::Available)
    );
    assert_eq!(
        cache.get((10_001, 42), start + Duration::from_secs(5)),
        None
    );
}

#[test]
fn group_ban_notices_update_bot_and_whole_group_mute_state() {
    let _serial = serialize_mute_cache_tests();
    let self_id = 91_001;
    let group_id = 92_001;
    group_mute_cache().lock().unwrap().remove_account(self_id);

    update_group_ban_notice(&json!({
        "post_type": "notice",
        "notice_type": "group_ban",
        "sub_type": "ban",
        "self_id": self_id,
        "group_id": group_id,
        "user_id": self_id,
        "duration": 120
    }));
    assert_eq!(
        group_mute_cache()
            .lock()
            .unwrap()
            .get((self_id, group_id), Instant::now()),
        Some(BotSendAvailability::Muted)
    );

    update_group_ban_notice(&json!({
        "post_type": "notice",
        "notice_type": "group_ban",
        "sub_type": "lift_ban",
        "self_id": self_id,
        "group_id": group_id,
        "user_id": self_id,
        "duration": 0
    }));
    assert_eq!(
        group_mute_cache()
            .lock()
            .unwrap()
            .get((self_id, group_id), Instant::now()),
        Some(BotSendAvailability::Available)
    );

    update_group_ban_notice(&json!({
        "post_type": "notice",
        "notice_type": "group_ban",
        "sub_type": "ban",
        "self_id": self_id,
        "group_id": group_id,
        "user_id": 0,
        "duration": 0
    }));
    assert_eq!(
        group_mute_cache()
            .lock()
            .unwrap()
            .get((self_id, group_id), Instant::now()),
        Some(BotSendAvailability::Muted)
    );
    group_mute_cache().lock().unwrap().remove_account(self_id);
}

#[tokio::test]
async fn bot_send_availability_queries_self_once_and_uses_the_cache() {
    let _serial = serialize_mute_cache_tests();
    let (handle, mut frames) = test_connection(None);
    let adapter = Arc::new(test_adapter(handle.clone(), Target::Group { group_id: 42 }));
    group_mute_cache()
        .lock()
        .unwrap()
        .remove_account(adapter.self_id);
    let lookup = {
        let adapter = adapter.clone();
        tokio::spawn(async move { adapter.bot_send_availability().await })
    };
    let frame: Value = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
    assert_eq!(frame["action"], "get_group_member_info");
    assert_eq!(frame["params"]["group_id"], 42);
    assert_eq!(frame["params"]["user_id"], adapter.self_id);
    // 09-11:这一条走了上游缓存,提前解禁之后问到的还是旧的 shut_up_timestamp,
    // 她在群里哑了两小时二十分。没有这条断言的话回滚不会报红。
    assert_eq!(frame["params"]["no_cache"], true);
    route_api_response(
        &handle,
        json!({
            "status": "ok",
            "retcode": 0,
            "data": {
                "group_id": 42,
                "user_id": adapter.self_id,
                "shut_up_timestamp": unix_now() + 60
            },
            "echo": frame["echo"]
        }),
    );
    assert_eq!(lookup.await.unwrap().unwrap(), BotSendAvailability::Muted);
    assert_eq!(
        adapter.bot_send_availability().await.unwrap(),
        BotSendAvailability::Muted
    );
    assert!(frames.try_recv().is_err());
    group_mute_cache()
        .lock()
        .unwrap()
        .remove_account(adapter.self_id);
}

/// 12 小时的禁言只信到复查窗口为止:提前解禁而通知没收到时,最坏 5 分钟自愈,
/// 而不是一路哑到原定解禁时刻。查询侧与通知侧都要守住。
#[tokio::test]
async fn a_long_mute_is_only_trusted_until_the_recheck_window() {
    let _serial = serialize_mute_cache_tests();
    let (handle, mut frames) = test_connection(None);
    let adapter = Arc::new(test_adapter(handle.clone(), Target::Group { group_id: 43 }));
    let key = (adapter.self_id, 43);
    group_mute_cache()
        .lock()
        .unwrap()
        .remove_account(adapter.self_id);

    let lookup = {
        let adapter = adapter.clone();
        tokio::spawn(async move { adapter.bot_send_availability().await })
    };
    let frame: Value = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
    route_api_response(
        &handle,
        json!({
            "status": "ok",
            "retcode": 0,
            "data": {
                "group_id": 43,
                "user_id": adapter.self_id,
                "shut_up_timestamp": unix_now() + 12 * 60 * 60
            },
            "echo": frame["echo"]
        }),
    );
    assert_eq!(lookup.await.unwrap().unwrap(), BotSendAvailability::Muted);

    let now = Instant::now();
    assert_eq!(
        group_mute_cache().lock().unwrap().get(key, now),
        Some(BotSendAvailability::Muted)
    );
    assert_eq!(
        group_mute_cache()
            .lock()
            .unwrap()
            .get(key, now + GROUP_MUTE_MUTED_TTL + Duration::from_secs(1)),
        None,
        "禁言状态过了复查窗口必须重新问一次"
    );

    group_mute_cache()
        .lock()
        .unwrap()
        .remove_account(adapter.self_id);
    update_group_ban_notice(&json!({
        "post_type": "notice",
        "notice_type": "group_ban",
        "sub_type": "ban",
        "self_id": adapter.self_id,
        "group_id": 43,
        "user_id": adapter.self_id,
        "duration": 12 * 60 * 60
    }));
    let now = Instant::now();
    assert_eq!(
        group_mute_cache().lock().unwrap().get(key, now),
        Some(BotSendAvailability::Muted)
    );
    assert_eq!(
        group_mute_cache()
            .lock()
            .unwrap()
            .get(key, now + GROUP_MUTE_MUTED_TTL + Duration::from_secs(1)),
        None,
        "通知给的禁言时长同样只是上界"
    );
    group_mute_cache()
        .lock()
        .unwrap()
        .remove_account(adapter.self_id);
}
