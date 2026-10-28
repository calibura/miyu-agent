//! 非消息事件：撤回、上传、禁言、进出群、好友申请。
//!
//! 这些事件和普通消息共用一条入口，靠 `is_*` 一族判定分流。判定写成独立函数
//! 而不是内联的 match，是因为同一个事件在不同 OneBot 实现里字段名不完全一致，
//! 兼容逻辑要有唯一的落点。
//!
//! 禁言通知会**回写缓存**（`update_group_ban_notice`）：被禁言时收到通知就直接
//! 更新，比等下次查询再发现要及时。

use crate::platforms::onebot::*;

pub(in crate::platforms::onebot) fn is_message_recall(event: &Value) -> bool {
    event.get("post_type").and_then(Value::as_str) == Some("notice")
        && matches!(
            event.get("notice_type").and_then(Value::as_str),
            Some("group_recall" | "friend_recall")
        )
}

pub(in crate::platforms::onebot) fn is_group_upload_notice(event: &Value) -> bool {
    event.get("post_type").and_then(Value::as_str) == Some("notice")
        && event.get("notice_type").and_then(Value::as_str) == Some("group_upload")
}

pub(in crate::platforms::onebot) fn is_friend_add_request(event: &Value) -> bool {
    event.get("post_type").and_then(Value::as_str) == Some("request")
        && event.get("request_type").and_then(Value::as_str) == Some("friend")
}

pub(in crate::platforms::onebot) fn is_group_request(event: &Value) -> bool {
    event.get("post_type").and_then(Value::as_str) == Some("request")
        && event.get("request_type").and_then(Value::as_str) == Some("group")
}

pub(in crate::platforms::onebot) fn is_group_add_request(event: &Value) -> bool {
    is_group_request(event) && event.get("sub_type").and_then(Value::as_str) == Some("add")
}

pub(in crate::platforms::onebot) fn is_group_invite_request(event: &Value) -> bool {
    is_group_request(event) && event.get("sub_type").and_then(Value::as_str) == Some("invite")
}

pub(in crate::platforms::onebot) fn is_group_ban_notice(event: &Value) -> bool {
    event.get("post_type").and_then(Value::as_str) == Some("notice")
        && event.get("notice_type").and_then(Value::as_str) == Some("group_ban")
}

pub(in crate::platforms::onebot) fn is_group_decrease_notice(event: &Value) -> bool {
    event.get("post_type").and_then(Value::as_str) == Some("notice")
        && event.get("notice_type").and_then(Value::as_str) == Some("group_decrease")
        && event.get("sub_type").and_then(Value::as_str) == Some("kick")
}

/// 戳一戳:OneBot v11 标准写法是 `notice_type=notify, sub_type=poke`
/// (NapCat / SnowLuma / LLOneBot 都是这个形状)。`notice_type=poke` 是少数
/// 实现的变体,一并认。
pub(in crate::platforms::onebot) fn is_poke_notice(event: &Value) -> bool {
    event.get("post_type").and_then(Value::as_str) == Some("notice")
        && (event.get("notice_type").and_then(Value::as_str) == Some("poke")
            || event.get("sub_type").and_then(Value::as_str) == Some("poke"))
}

/// 账号自身上下线。SnowLuma 送 `bot_status`(sub_type 为 `online`/`offline`),
/// 被踢下线仍是 `bot_offline`。
pub(in crate::platforms::onebot) fn is_bot_status_notice(event: &Value) -> bool {
    event.get("post_type").and_then(Value::as_str) == Some("notice")
        && matches!(
            event.get("notice_type").and_then(Value::as_str),
            Some("bot_status" | "bot_offline")
        )
}

/// 群成员增加(`group_increase`)。自己进群也走这条。
pub(in crate::platforms::onebot) fn is_group_increase_notice(event: &Value) -> bool {
    event.get("post_type").and_then(Value::as_str) == Some("notice")
        && event.get("notice_type").and_then(Value::as_str) == Some("group_increase")
}

pub(in crate::platforms::onebot) fn update_group_ban_notice(event: &Value) {
    let self_id = event.get("self_id").and_then(Value::as_i64).unwrap_or(0);
    let group_id = event.get("group_id").and_then(Value::as_i64).unwrap_or(0);
    let user_id = event.get("user_id").and_then(Value::as_i64).unwrap_or(-1);
    if self_id == 0 || group_id == 0 || !matches!(user_id, 0) && user_id != self_id {
        return;
    }
    let duration = event.get("duration").and_then(Value::as_u64).unwrap_or(0);
    let sub_type = event.get("sub_type").and_then(Value::as_str);
    if user_id == 0 && duration == 0 && !matches!(sub_type, Some("ban" | "lift_ban")) {
        return;
    }
    let lifted = sub_type == Some("lift_ban") || user_id != 0 && duration == 0;
    let now = Instant::now();
    let (availability, ttl) = if lifted {
        (BotSendAvailability::Available, GROUP_MUTE_AVAILABLE_TTL)
    } else {
        (
            BotSendAvailability::Muted,
            if duration == 0 {
                GROUP_MUTE_WHOLE_NOTICE_TTL
            } else {
                // 上界是禁言时长,但到点要复查——提前解禁的通知可能收不到。
                Duration::from_secs(duration).min(GROUP_MUTE_MUTED_TTL)
            },
        )
    };
    group_mute_cache()
        .lock()
        .unwrap()
        .insert((self_id, group_id), availability, ttl, now);
}

pub(in crate::platforms::onebot) fn recall_event(
    target: Target,
    event: &Value,
    user_id: i64,
) -> PlatformInboundEvent {
    let self_id = event.get("self_id").and_then(Value::as_i64).unwrap_or(0);
    PlatformInboundEvent {
        kind: PlatformInboundEventKind::MessageRecall,
        message_seq: None,
        conversation: platform_conversation(target, self_id),
        conversation_display_name: None,
        message_id: event
            .get("message_id")
            .and_then(value_id_string)
            .unwrap_or_default(),
        sender_id: user_id.to_string(),
        sender_display_name: event_sender_display_name(event),
        operator_id: event.get("operator_id").and_then(value_id_string),
        timestamp: event.get("time").and_then(Value::as_i64).unwrap_or(0),
        received_at: Instant::now(),
        message_position: None,
        ingress_order: None,
        text: String::new(),
        reply_to_message_id: None,
        replied_message: None,
        mentioned_user_ids: Vec::new(),
        mentioned_users: Vec::new(),
        mentioned_bot: false,
        media: Vec::new(),
        notice_sub_type: event
            .get("sub_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        duration_seconds: None,
    }
}

pub(in crate::platforms::onebot) fn group_upload_event(
    event: &Value,
) -> Option<PlatformInboundEvent> {
    let self_id = event.get("self_id").and_then(Value::as_i64)?;
    let group_id = event.get("group_id").and_then(Value::as_i64)?;
    let user_id = event.get("user_id").and_then(Value::as_i64)?;
    let file = event.get("file")?;
    let file_id = file.get("id").and_then(Value::as_str).map(str::trim);
    let file_name = file
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if self_id == 0 || group_id == 0 || user_id == 0 {
        return None;
    }
    let Some(file_id) = file_id.filter(|id| !id.is_empty()) else {
        return None;
    };
    let media_id = file_id.to_string();
    let message_id = format!(
        "group_file_{}",
        file_id
            .trim_start_matches('/')
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>()
    );
    Some(PlatformInboundEvent {
        kind: PlatformInboundEventKind::GroupFileUpload,
        message_seq: None,
        conversation: platform_conversation(Target::Group { group_id }, self_id),
        conversation_display_name: None,
        message_id,
        sender_id: user_id.to_string(),
        sender_display_name: String::new(),
        operator_id: None,
        timestamp: event.get("time").and_then(Value::as_i64).unwrap_or(0),
        received_at: Instant::now(),
        message_position: None,
        ingress_order: Some(next_ingress_order()),
        text: String::new(),
        reply_to_message_id: None,
        replied_message: None,
        mentioned_user_ids: Vec::new(),
        mentioned_users: Vec::new(),
        mentioned_bot: false,
        media: vec![PlatformInboundMedia {
            kind: PlatformMediaKind::File,
            id: Some(media_id),
            name: file_name.map(str::to_string),
            url: None,
        }],
        notice_sub_type: None,
        duration_seconds: None,
    })
}

pub(in crate::platforms::onebot) async fn handle_group_file_upload(
    state: DaemonState,
    conn: ConnectionHandle,
    event: Value,
) {
    let app_config = state.manager.lock().unwrap().config.clone();
    let config = &app_config.platforms.qq;
    if !config.enabled {
        return;
    }
    let self_id = event.get("self_id").and_then(Value::as_i64).unwrap_or(0);
    let user_id = event.get("user_id").and_then(Value::as_i64).unwrap_or(0);
    let Some(group_id) = event
        .get("group_id")
        .and_then(Value::as_i64)
        .filter(|group_id| *group_id != 0)
    else {
        return;
    };
    let Some(mut inbound) = group_upload_event(&event) else {
        return;
    };
    let target = Target::Group { group_id };
    if !admission_for_with_state(config, &state.state_store, target, self_id, user_id).allowed {
        return;
    }
    let context = match platform_turn_context(
        &state,
        conn,
        target,
        &event,
        app_config,
        Some(inbound.clone()),
    ) {
        Ok(context) => context,
        Err(error) => {
            tracing::warn!(target: "miyu::qq", error = %error, "{}", t("OneBot group-file observer initialization failed", "OneBot 群文件观察器初始化失败"));
            return;
        }
    };
    if inbound.sender_display_name.trim().is_empty() {
        if let Ok(Some(member)) = context.group_member(&inbound.sender_id).await {
            inbound.sender_display_name = member.display_name().trim().to_string();
        }
    }
    context.observe_inbound(&inbound).await;
}

pub(in crate::platforms::onebot) fn group_management_notice(
    event: &Value,
) -> Option<PlatformInboundEvent> {
    let self_id = event.get("self_id").and_then(Value::as_i64)?;
    let group_id = event.get("group_id").and_then(Value::as_i64)?;
    let user_id = event.get("user_id").and_then(Value::as_i64)?;
    let kind = match event.get("notice_type").and_then(Value::as_str)? {
        "group_ban" => PlatformInboundEventKind::GroupBan,
        "group_decrease" => PlatformInboundEventKind::GroupDecrease,
        _ => return None,
    };
    if self_id == 0 || group_id == 0 || user_id == 0 {
        return None;
    }
    Some(PlatformInboundEvent {
        kind,
        conversation: platform_conversation(Target::Group { group_id }, self_id),
        conversation_display_name: None,
        message_id: String::new(),
        message_seq: None,
        sender_id: user_id.to_string(),
        sender_display_name: user_id.to_string(),
        operator_id: event.get("operator_id").and_then(value_id_string),
        timestamp: event.get("time").and_then(Value::as_i64).unwrap_or(0),
        received_at: Instant::now(),
        message_position: None,
        ingress_order: None,
        text: String::new(),
        reply_to_message_id: None,
        replied_message: None,
        mentioned_user_ids: Vec::new(),
        mentioned_users: Vec::new(),
        mentioned_bot: false,
        media: Vec::new(),
        notice_sub_type: event
            .get("sub_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        duration_seconds: event.get("duration").and_then(Value::as_u64),
    })
}

pub(in crate::platforms::onebot) async fn handle_group_management_notice(
    state: DaemonState,
    conn: ConnectionHandle,
    event: Value,
) {
    let Some(inbound) = group_management_notice(&event) else {
        return;
    };
    let config = state.manager.lock().unwrap().config.clone();
    if !config.platforms.qq.enabled {
        return;
    }
    let group_id = inbound
        .conversation
        .conversation_id
        .parse::<i64>()
        .unwrap_or(0);
    let user_id = inbound.sender_id.parse::<i64>().unwrap_or(0);
    let self_id = inbound.conversation.account_id.parse::<i64>().unwrap_or(0);
    let target = Target::Group { group_id };
    if !admission_for_with_state(
        &config.platforms.qq,
        &state.state_store,
        target,
        self_id,
        user_id,
    )
    .allowed
    {
        return;
    }
    match platform_turn_context(&state, conn, target, &event, config, Some(inbound.clone())) {
        Ok(context) => context.observe_inbound(&inbound).await,
        Err(error) => {
            tracing::warn!(target: "miyu::qq", error = %error, "{}", t("OneBot group notice observer initialization failed", "OneBot 群通知观察器初始化失败"))
        }
    }
}

pub(in crate::platforms::onebot) async fn handle_poke(
    state: DaemonState,
    conn: ConnectionHandle,
    event: Value,
) {
    // 只有戳到机器人才往下走:群里 A 戳 B 与 Miyu 无关。
    let self_id = event.get("self_id").and_then(value_i64).unwrap_or(0);
    let from_id = poke_poker_id(&event).unwrap_or(0);
    if self_id == 0 || from_id == 0 {
        return;
    }
    let poked_id = poke_target_id(&event).unwrap_or(from_id);
    // info 级保留：戳一戳是低频事件（一天几十次），而且它曾经因为缺 @ 段被
    // 静默丢掉——那类「事件到了却没反应」的问题，有这一行就能一眼分辨是
    // 「没收到」还是「收到没处理」。
    tracing::info!(
        target: "miyu::qq",
        self_id,
        from_id,
        poked_id,
        group_id = event.get("group_id").and_then(value_i64).unwrap_or(0),
        "{}",
        t("OneBot poke received", "OneBot 收到戳一戳")
    );
    if poked_id != self_id {
        return;
    }
    // 改写成一条合成的文本消息走完整调度链。理由:戳一戳在语义上就是「对你
    // 说了一句话」,而限流、会话排队、上下文拼装、主动判断、工具面、落库
    // 全都在 `handle_message_with_activity` 那一条路上;另起一条旁路等于把
    // 那些逐个重写一遍,且迟早与主路分叉。
    let synthetic = poke_as_message(&event);
    let Some(synthetic) = synthetic else {
        return;
    };
    handle_message_with_activity(state, conn, synthetic, next_ingress_order(), None).await;
}

/// 一次戳一戳里「动手的人」是谁。
///
/// SnowLuma 的群戳与私聊戳字段语义**不一样**:群戳 `user_id` 就是动手的人;
/// 私聊戳 `user_id` 是会话对端(也就是机器人自己),真正动手的人在 `sender_id`
/// 里(见其 `convertFriendPoke` / `convertGroupPoke`)。取了错的那个,私聊戳
/// 会被当成「机器人自己发的」而在调度入口丢掉。
pub(in crate::platforms::onebot) fn poke_poker_id(event: &Value) -> Option<i64> {
    event
        .get("sender_id")
        .and_then(value_i64)
        .or_else(|| event.get("user_id").and_then(value_i64))
        .filter(|id| *id != 0)
}

/// 被戳的人。缺省时按动手的人算(少数实现不带 `target_id`)。
pub(in crate::platforms::onebot) fn poke_target_id(event: &Value) -> Option<i64> {
    event
        .get("target_id")
        .and_then(value_i64)
        .filter(|id| *id != 0)
        .or_else(|| poke_poker_id(event))
}

/// 把戳一戳改写成一条 OneBot 群/私聊消息事件。
///
/// 文本取 `suffix`(QQ 的戳一戳常带一句「拍了拍我的 xx」),没有就用一句
/// 中性的动作描述——模型需要知道「这是一次戳,不是一次普通的说话」。
pub(in crate::platforms::onebot) fn poke_as_message(event: &Value) -> Option<Value> {
    let self_id = event.get("self_id").and_then(value_i64)?;
    let from_id = poke_poker_id(event)?;
    if self_id == 0 || from_id == 0 {
        return None;
    }
    let group_id = event
        .get("group_id")
        .and_then(value_i64)
        .filter(|group_id| *group_id != 0);
    let suffix = event
        .get("suffix")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let text = if suffix.is_empty() {
        "（戳了戳你）".to_string()
    } else {
        format!("（戳了戳你：{suffix}）")
    };
    // `message_id` / `real_seq` 必须成对出现:下游按平台序号排到达顺序、做引用
    // 与撤回的定位,缺了序号整条会被当成"悄悄消失"的消息丢掉(实测)。戳一戳本身
    // 没有消息 id,这里自造一对——值是合成的,只用于本进程内的排序与幂等。
    let stamp = event.get("time").and_then(Value::as_i64).unwrap_or(0);
    let synthetic_id = next_ingress_order();
    let mut message = json!({
        "post_type": "message",
        "self_id": self_id,
        "user_id": from_id,
        "message_type": if group_id.is_some() { "group" } else { "private" },
        "message_id": synthetic_id,
        "message_seq": synthetic_id,
        "real_seq": synthetic_id.to_string(),
        // 必须带一个 `at` 机器人自己的段:群聊里 `group_trigger_text` 只认
        // 「@ 了我」「引用了我」「唤醒词开头」三种唤醒条件,戳一戳三者都不满足
        // ——不加这一段的戳会被当成普通闲聊静默丢掉(实测:进了回合队列却再
        // 无下文)。戳在语义上就是「直接叫到你」,与 @ 等价。
        "message": [
            { "type": "at", "data": { "qq": self_id.to_string() } },
            { "type": "text", "data": { "text": text } }
        ],
        "raw_message": text,
        "time": stamp,
    });
    if let Some(group_id) = group_id {
        message["group_id"] = json!(group_id);
    }
    if let Some(sender) = event.get("sender") {
        message["sender"] = sender.clone();
    }
    Some(message)
}

/// 账号自身上下线。掉线/被踢时把事实写进日志,好过让上层从 WS 断开去猜。
pub(in crate::platforms::onebot) fn handle_bot_status(event: &Value) {
    let self_id = event.get("self_id").and_then(value_i64).unwrap_or(0);
    let notice = event.get("notice_type").and_then(Value::as_str).unwrap_or("");
    let sub_type = event.get("sub_type").and_then(Value::as_str).unwrap_or("");
    let offline = notice == "bot_offline" || sub_type == "offline";
    if offline {
        tracing::warn!(
            target: "miyu::qq",
            self_id,
            notice,
            sub_type,
            "{}",
            t("Tencent QQ account went offline", "腾讯 QQ 账号已下线")
        );
    } else {
        tracing::info!(
            target: "miyu::qq",
            self_id,
            notice,
            sub_type,
            "{}",
            t("Tencent QQ account is online", "腾讯 QQ 账号已上线")
        );
    }
}

/// 群成员增加。自己进群时顺手把群名刷进缓存,免得第一条消息带着空群名。
pub(in crate::platforms::onebot) fn group_increase_event(
    event: &Value,
) -> Option<PlatformInboundEvent> {
    let self_id = event.get("self_id").and_then(value_i64)?;
    let group_id = event.get("group_id").and_then(value_i64)?;
    let user_id = event.get("user_id").and_then(value_i64)?;
    if self_id == 0 || group_id == 0 || user_id == 0 {
        return None;
    }
    Some(PlatformInboundEvent {
        kind: PlatformInboundEventKind::GroupIncrease,
        message_seq: None,
        conversation: platform_conversation(Target::Group { group_id }, self_id),
        conversation_display_name: None,
        message_id: String::new(),
        sender_id: user_id.to_string(),
        sender_display_name: event_sender_display_name(event),
        operator_id: event.get("operator_id").and_then(value_id_string),
        timestamp: event.get("time").and_then(Value::as_i64).unwrap_or(0),
        received_at: Instant::now(),
        message_position: None,
        ingress_order: None,
        text: String::new(),
        reply_to_message_id: None,
        replied_message: None,
        mentioned_user_ids: Vec::new(),
        mentioned_users: Vec::new(),
        mentioned_bot: user_id == self_id,
        media: Vec::new(),
        notice_sub_type: event
            .get("sub_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        duration_seconds: None,
    })
}

pub(in crate::platforms::onebot) async fn handle_message_recall(
    state: DaemonState,
    conn: ConnectionHandle,
    event: Value,
) {
    let app_config = state.manager.lock().unwrap().config.clone();
    let config = &app_config.platforms.qq;
    if !config.enabled {
        return;
    }
    let self_id = event.get("self_id").and_then(Value::as_i64).unwrap_or(0);
    let user_id = event.get("user_id").and_then(Value::as_i64).unwrap_or(0);
    if self_id == 0 || user_id == 0 {
        return;
    }
    let target = match event.get("notice_type").and_then(Value::as_str) {
        Some("group_recall") => event
            .get("group_id")
            .and_then(Value::as_i64)
            .filter(|group_id| *group_id != 0)
            .map(|group_id| Target::Group { group_id }),
        Some("friend_recall") => Some(Target::Private { user_id }),
        _ => None,
    };
    let Some(target) = target else { return };
    if !admission_for_with_state(config, &state.state_store, target, self_id, user_id).allowed {
        return;
    }
    let inbound = recall_event(target, &event, user_id);
    let context = match platform_turn_context(
        &state,
        conn,
        target,
        &event,
        app_config,
        Some(inbound.clone()),
    ) {
        Ok(context) => context,
        Err(error) => {
            tracing::warn!(target: "miyu::qq", error = %error, "{}", t("OneBot recall observer initialization failed", "OneBot 撤回观察器初始化失败"));
            return;
        }
    };
    context.observe_inbound(&inbound).await;
}
