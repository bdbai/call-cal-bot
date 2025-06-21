use std::fs;

use chrono::prelude::*;
use mania::entity::bot_group_member::BotGroupMember;
use mania::event::group::GroupEvent;
use mania::event::group::group_message::GroupMessageEvent;
use mania::message::builder::MessageChainBuilder;
use mania::message::chain::{GroupMessageUniqueElem, MessageChain, MessageType};
use mania::message::entity::{Entity, Mention};
use mania::{Client, ClientConfig, DeviceInfo, KeyStore};
use rusqlite::{Connection, params};
use tracing::{debug, error};

refinery::embed_migrations!("migrations");

const BOT_TZ: FixedOffset = FixedOffset::east_opt(8 * 3600).expect("UTC+8 offset");
const BOT_CHECKPOINT: NaiveTime =
    NaiveTime::from_hms_opt(4, 0, 0).expect("Valid time for bot checkpoint");

struct BotContext<'a> {
    conn: &'a mut Connection,
}

pub fn init_db(conn: &mut Connection) {
    migrations::runner().run(conn).expect("db migration");
}

fn get_checkpoint_timestamp() -> i64 {
    get_checkpoint().timestamp()
}
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

    let rows = match stmt.query_map(params![checkpoint_start, checkpoint_end], |row| {
        let nickname: String = row.get(0)?;
        let created_at: Option<String> = row.get(1)?;
        let has_record = created_at.is_some();
        Ok((nickname, has_record))
    }) {
        Ok(rows) => rows,
        Err(e) => {
            error!("Failed to query daily report: {:?}", e);
            return "打卡日报查询失败".to_string();
        }
    };
    let rows: Result<Vec<_>, _> = rows.collect();
    let mut rows = match rows {
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
        .into_iter()
        .map(|(row_text, _)| row_text)
        .collect::<Vec<_>>()
        .join("\u{3000}");
    let rows_wo_record = rows_wo_record
        .into_iter()
        .map(|(row_text, _)| row_text)
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
    let checkpoint = get_checkpoint_timestamp();

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

    let res = 我没打卡_stmt.execute(params![user_id, checkpoint]);
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
    let checkpoint = get_checkpoint_timestamp();

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

    let res = 打卡_stmt.execute(params![user_id, checkpoint]);
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

fn handle_group_msg(ctx: &mut BotContext<'_>, ev: &GroupMessageEvent) -> Option<MessageChain> {
    let MessageType::Group(GroupMessageUniqueElem {
        group_uin,
        group_member_info: Some(group_member_info),
    }) = &ev.chain.typ
    else {
        return None;
    };
    debug!("group_member_info: {:?}", group_member_info);

    let text_entities = ev.chain.entities.iter().filter_map(|e| {
        if let Entity::Text(te) = e {
            Some(te.text.trim())
        } else {
            None
        }
    });
    let first_text = text_entities
        .filter(|te| !te.is_empty())
        .next()
        .unwrap_or_default();
    let (command, args) = first_text.split_once(' ').unwrap_or((first_text, ""));
    match command {
        "/打卡" => handle_打卡(ctx, *group_uin, group_member_info, args),
        "/我没打卡" => handle_我没打卡(ctx, *group_uin, group_member_info, args),
        "/今日" => {
            let report = build_daily_report(ctx);
            Some(MessageChainBuilder::group(*group_uin).text(&report).build())
        }
        "/打卡改名" => todo!(),
        _ => return None,
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let mut conn = Connection::open("call-cal-bot.db").expect("Failed to open database");
    init_db(&mut conn);

    let config = ClientConfig::default();
    let device = DeviceInfo::load("device.json").unwrap_or_else(|_| {
        tracing::warn!("Failed to load device info, generating a new one...");
        let device = DeviceInfo::default();
        device.save("device.json").unwrap();
        device
    });
    let key_store = KeyStore::load("keystore.json").unwrap_or_else(|_| {
        tracing::warn!("Failed to load keystore, generating a new one...");
        let key_store = KeyStore::default();
        key_store.save("keystore.json").unwrap();
        key_store
    });
    let need_login = key_store.is_expired();
    let mut client = Client::new(config, device, key_store).await.unwrap();

    let op = client.handle().operator().clone();
    let send_op = client.handle().operator().clone();
    let mut group_receiver = op.event_listener.group.clone();
    let mut system_receiver = op.event_listener.system.clone();

    tokio::spawn(async move {
        let mut ctx = BotContext { conn: &mut conn };
        loop {
            let mut reply = None;
            tokio::select! {
                _ = system_receiver.changed() => {
                    if let Some(ref se) = *system_receiver.borrow() {
                        tracing::info!("[SystemEvent] {:?}", se);
                    }
                }
                _ = group_receiver.changed() => {
                    let guard = group_receiver.borrow();
                    if let Some(ref ge) = *guard {
                        tracing::debug!("[GroupEvent] {:?}", ge);
                        match ge {
                            GroupEvent::GroupMessage(gme) => {
                                if let mania::message::chain::MessageType::Group(_gmeu) = &gme.chain.typ {
                                    reply = handle_group_msg(&mut ctx, gme);
                                }
                            }
                            _ => {},
                        }
                    }
                }
            }
            if let Some(chain) = reply {
                tracing::debug!("Replying with message chain: {:?}", chain);
                if let Err(e) = send_op.send_message(chain).await {
                    tracing::error!("Failed to send message: {:?}", e);
                }
            }
        }
    });

    tokio::spawn(async move {
        client.spawn().await;
    });

    if need_login {
        tracing::warn!("Session is invalid, need to login again!");
        let login_res: Result<(), String> = async {
            let (url, bytes) = op.fetch_qrcode().await.map_err(|e| e.to_string())?;
            let qr_code_name = format!("qrcode.png");
            fs::write(&qr_code_name, &bytes).map_err(|e| e.to_string())?;
            tracing::info!(
                "QR code fetched successfully! url: {}, saved to {}",
                url,
                qr_code_name
            );
            let login_res = op.login_by_qrcode().await.map_err(|e| e.to_string());
            match fs::remove_file(&qr_code_name).map_err(|e| e.to_string()) {
                Ok(_) => tracing::info!("QR code file {} deleted successfully", qr_code_name),
                Err(e) => tracing::error!("Failed to delete QR code file {}: {}", qr_code_name, e),
            }
            login_res
        }
        .await;
        if let Err(e) = login_res {
            panic!("Failed to login: {e:?}");
        }
    } else {
        tracing::info!("Session is still valid, trying to online...");
    }

    let _tx = match op.online().await {
        Ok(tx) => tx,
        Err(e) => {
            panic!("Failed to set online status: {e:?}");
        }
    };
    tracing::info!("Bot online");

    op.update_key_store()
        .save("keystore.json")
        .unwrap_or_else(|e| tracing::error!("Failed to save key store: {:?}", e));

    tokio::signal::ctrl_c().await.unwrap();
}
