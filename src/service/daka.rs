use chrono::prelude::*;
use rusqlite::params;
use tracing::{debug, error};

const BOT_TZ: FixedOffset = FixedOffset::east_opt(8 * 3600).expect("UTC+8 offset");
const BOT_CHECKPOINT: NaiveTime =
    NaiveTime::from_hms_opt(4, 0, 0).expect("Valid time for bot checkpoint");

/// Get the datetime at 4 AM of the current day if the current time is after 4 AM,
/// otherwise get the datetime at 4 AM of the previous day. Use UTC+8 time zone.
fn get_checkpoint() -> DateTime<FixedOffset> {
    let now = BOT_TZ.from_utc_datetime(&Utc::now().naive_utc());
    let checkpoint_date = if now.time() >= BOT_CHECKPOINT {
        now.date_naive()
    } else {
        now.date_naive().pred_opt().expect("Valid prev date")
    };

    BOT_TZ
        .from_local_datetime(&NaiveDateTime::new(checkpoint_date, BOT_CHECKPOINT))
        .single()
        .expect("Valid checkpoint datetime")
}

fn build_daily_report(ctx: &mut BotContext<'_>) -> String {
    let checkpoint_start = get_checkpoint();
    let checkpoint_end = checkpoint_start + chrono::Duration::days(1);
    let mut stmt = ctx
        .conn
        .prepare_cached(
            // Get all member nicknames and their daka time (if exists) within
            // the two checkpoints
            "SELECT `bot_group_member`.`group_nickname`, D.`created_at` FROM `bot_group_member`
            LEFT JOIN (
                SELECT `created_at`, `user_id` FROM `bot_daka` WHERE `bot_daka`.`created_at` >= ?1 AND `bot_daka`.`created_at` < ?2
            ) D ON D.`user_id` = `bot_group_member`.`id`
            ORDER BY D.`created_at` ASC, `bot_group_member`.`sort_key` ASC, `bot_group_member`.`id` ASC",
        )
        .expect("Prepare statement failed");

    let rows = match stmt.query_map(
        params![checkpoint_start.naive_utc(), checkpoint_end.naive_utc()],
        |row| {
            let nickname: String = row.get(0)?;
            let created_at: Option<String> = row.get(1)?;
            let has_record = created_at.is_some();
            Ok((nickname, has_record))
        },
    ) {
        Ok(rows) => rows,
        Err(e) => {
            error!("Failed to query daily report: {:?}", e);
            return "打卡日报查询失败".to_string();
        }
    };
    let rows: Result<Vec<_>, _> = rows.collect();
    let rows = match rows {
        Ok(rows) => rows,
        Err(e) => {
            error!("Failed to collect daily report rows: {:?}", e);
            return "打卡日报查询失败".to_string();
        }
    };

    let (rows_has_record, rows_wo_record): (Vec<_>, _) =
        rows.into_iter().partition(|(_, has_record)| *has_record);
    if rows_has_record.is_empty() {
        return "今日无人打卡".to_string();
    }

    let rows_has_record = rows_has_record
        .iter()
        .map(|(row_text, _)| &**row_text)
        .collect::<Vec<_>>()
        .join("\u{3000}");
    let rows_wo_record = rows_wo_record
        .iter()
        .map(|(row_text, _)| &**row_text)
        .collect::<Vec<_>>()
        .join("\u{3000}");
    format!(
        "{}/{}\n{rows_has_record} ✅\n{rows_wo_record} ❌",
        checkpoint_start.month(),
        checkpoint_start.day()
    )
}

fn handle_我没打卡(
    ctx: &mut BotContext<'_>,
    group_uin: u32,
    group_member_info: &BotGroupMember,
    _args: &str,
) -> Option<MessageChain> {
    let uin = group_member_info.uin;
    let uid = &*group_member_info.uid;
    let nickname = group_member_info.member_name.as_deref().unwrap_or_default();
    let group_nickname = group_member_info.member_card.as_deref().unwrap_or(nickname);

    debug!("Handling 我没打卡 command for user {}", uin);
    let checkpoint = get_checkpoint();

    // TODO: refactor
    const UPSERT_RECORD_SQL: &str =
        "INSERT INTO `bot_group_member` (`qq_uid`, `qq_uin`, `nickname`, `group_nickname`)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT (`qq_uid`)
            DO UPDATE SET `nickname` = excluded.nickname, `group_nickname` = excluded.group_nickname
        RETURNING `id`";
    let mut update_member_name_stmt = ctx
        .conn
        .prepare_cached(UPSERT_RECORD_SQL)
        .expect("Prepare statement failed");
    let user_id: i64 = match update_member_name_stmt
        .query_row(params![uid, uin, nickname, group_nickname], |row| {
            row.get(0)
        }) {
        Ok(id) => id,
        Err(e) => {
            error!("Failed to update member name: {:?}", e);
            return Some(
                MessageChainBuilder::group(group_uin)
                    .text("打卡失败：无法更新用户信息")
                    .build(),
            );
        }
    };
    drop(update_member_name_stmt);

    let mut 我没打卡_stmt = ctx
        .conn
        .prepare_cached("DELETE FROM `bot_daka` WHERE `user_id` = ?1 AND `created_at` >= ?2")
        .expect("Prepare statement failed");

    let res = 我没打卡_stmt.execute(params![user_id, checkpoint.naive_utc()]);
    drop(我没打卡_stmt);
    let msg = match res {
        Ok(0) => "确实",
        Ok(_) => "行吧",
        Err(e) => {
            tracing::error!("Failed to insert record: {:?}", e);
            "我没打卡失败：数据库错误"
        }
    };

    let mut reply_chain = MessageChainBuilder::group(group_uin);
    reply_chain.text(" ").text(msg);
    let mut chain = reply_chain.build();
    chain.entities.insert(
        0,
        Entity::Mention(Mention {
            uid: uid.into(),
            name: Some(format!("@{group_nickname}")),
            uin,
        }),
    );
    Some(chain)
}
fn handle_打卡(
    ctx: &mut BotContext<'_>,
    group_uin: u32,
    group_member_info: &BotGroupMember,
    _args: &str,
) -> Option<MessageChain> {
    let uin = group_member_info.uin;
    let uid = &*group_member_info.uid;
    let nickname = group_member_info.member_name.as_deref().unwrap_or_default();
    let group_nickname = group_member_info.member_card.as_deref().unwrap_or(nickname);

    debug!("Handling 打卡 command for user {}", uin);
    let checkpoint = get_checkpoint();

    const UPSERT_RECORD_SQL: &str =
        "INSERT INTO `bot_group_member` (`qq_uid`, `qq_uin`, `nickname`, `group_nickname`)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT (`qq_uid`)
            DO UPDATE SET `nickname` = excluded.nickname, `group_nickname` = excluded.group_nickname
        RETURNING `id`";
    let mut update_member_name_stmt = ctx
        .conn
        .prepare_cached(UPSERT_RECORD_SQL)
        .expect("Prepare statement failed");
    let user_id: i64 = match update_member_name_stmt
        .query_row(params![uid, uin, nickname, group_nickname], |row| {
            row.get(0)
        }) {
        Ok(id) => id,
        Err(e) => {
            error!("Failed to update member name: {:?}", e);
            return Some(
                MessageChainBuilder::group(group_uin)
                    .text("打卡失败：无法更新用户信息")
                    .build(),
            );
        }
    };
    drop(update_member_name_stmt);

    let mut 打卡_stmt = ctx
        .conn
        .prepare_cached(
            "INSERT INTO `bot_daka` (`user_id`) SELECT ?1 WHERE NOT EXISTS (
            SELECT 1 FROM `bot_daka` WHERE `user_id` = ?1 AND `created_at` >= ?2
        )",
        )
        .expect("Prepare statement failed");

    let res = 打卡_stmt.execute(params![user_id, checkpoint.naive_utc()]);
    drop(打卡_stmt);
    let mut _daily_report = String::new();
    let msg = match res {
        Ok(0) => "您今天已经打过卡莉",
        Ok(_) => {
            _daily_report = build_daily_report(ctx);
            &_daily_report
        }
        Err(e) => {
            tracing::error!("Failed to insert record: {:?}", e);
            "打卡失败：数据库错误"
        }
    };

    let mut reply_chain = MessageChainBuilder::group(group_uin);
    reply_chain.text(" ").text(msg);
    let mut chain = reply_chain.build();
    chain.entities.insert(
        0,
        Entity::Mention(Mention {
            uid: uid.into(),
            name: Some(format!("@{group_nickname}")),
            uin,
        }),
    );
    Some(chain)
}

fn handle_咕(
    ctx: &mut BotContext<'_>,
    group_uin: u32,
    group_member_info: &BotGroupMember,
    _args: &str,
) -> Option<MessageChain> {
    let uin = group_member_info.uin;

    debug!("Handling 咕 command for user {}", uin);
    let checkpoint_end = get_checkpoint();
    let checkpoint_start = checkpoint_end - chrono::Duration::days(10);

    let mut get_records_10day_stmt = ctx
        .conn
        .prepare_cached(
            "SELECT
                `bot_group_member`.`group_nickname`,
                (
                    SELECT `created_at` FROM `bot_daka`
                    WHERE `bot_daka`.`user_id` = `bot_group_member`.`id`
                    AND `bot_daka`.`created_at` >= ?1 AND `bot_daka`.`created_at` < ?2
                    ORDER BY `bot_daka`.`id` DESC LIMIT 1
                ) AS `last_daka_at`
            FROM `bot_group_member`
            ORDER BY `bot_group_member`.`sort_key` ASC, `bot_group_member`.`id` ASC",
        )
        .expect("Prepare statement failed");

    #[derive(Debug, Clone)]
    struct DakaRecord {
        group_nickname: String,
        last_daka_at: Option<DateTime<FixedOffset>>,
    }
    let res = get_records_10day_stmt
        .query_map(
            params![checkpoint_start.naive_utc(), checkpoint_end.naive_utc()],
            |row| {
                let group_nickname: String = row.get(0)?;
                let created_at: Option<DateTime<Utc>> = row.get(1)?;
                Ok(DakaRecord {
                    group_nickname,
                    last_daka_at: created_at.map(|dt| dt.with_timezone(&BOT_TZ)),
                })
            },
        )
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>());
    drop(get_records_10day_stmt);

    let records = match res {
        Ok(rows) => rows,
        Err(e) => {
            error!("Failed to query records: {:?}", e);
            return Some(
                MessageChainBuilder::group(group_uin)
                    .text("咕咕查询失败：数据库错误")
                    .build(),
            );
        }
    };

    let failed_group_members = records
        .iter()
        .filter(|record| {
            if let Some(last_daka_at) = record.last_daka_at {
                last_daka_at < checkpoint_start
            } else {
                true
            }
        })
        .map(|record| &*record.group_nickname)
        .collect::<Vec<_>>();
    let failed_msg = if failed_group_members.is_empty() {
        "".into()
    } else {
        format!("💢 10天没打卡：\n{}", failed_group_members.join("\u{3000}"))
    };

    let warning_checkpoint = checkpoint_end - chrono::Duration::days(7);
    let warning_group_members = records
        .iter()
        .filter_map(|record| {
            (record.last_daka_at? < warning_checkpoint).then_some(&*record.group_nickname)
        })
        .collect::<Vec<_>>();
    let warning_msg = if warning_group_members.is_empty() {
        "".into()
    } else {
        format!("⚠️ 7天没打卡：\n{}", warning_group_members.join("\u{3000}"))
    };

    let msg = if failed_msg.is_empty() && warning_msg.is_empty() {
        "没有人咕咕".to_string()
    } else {
        format!("{}\n{}", failed_msg, warning_msg)
    };

    let chain = MessageChainBuilder::group(group_uin)
        .text(" ")
        .text(msg.trim())
        .build();
    Some(chain)
}
