use crate::service::models::{GroupMember, ServiceResponse};
use chrono::prelude::*;
use rusqlite::params;
use tracing::error;

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

impl super::Service {
    pub fn build_daily_report(&self) -> String {
        let checkpoint_start = get_checkpoint();
        let checkpoint_end = checkpoint_start + chrono::Duration::days(1);

        // Lock connection for this query
        let conn_guard = self.conn.lock().unwrap();
        let mut stmt = match conn_guard.prepare_cached(
            // Get all member nicknames and their daka time (if exists) within
            // the two checkpoints
            "SELECT `bot_group_member`.`group_nickname`, D.`created_at` FROM `bot_group_member`
            LEFT JOIN (
                SELECT `created_at`, `user_id` FROM `bot_daka` WHERE `bot_daka`.`created_at` >= ?1 AND `bot_daka`.`created_at` < ?2
            ) D ON D.`user_id` = `bot_group_member`.`id`
            ORDER BY D.`created_at` ASC, `bot_group_member`.`sort_key` ASC, `bot_group_member`.`id` ASC",
        ) {
            Ok(s) => s,
            Err(e) => {
                error!("Prepare statement failed: {:?}", e);
                return "打卡日报查询失败".to_string();
            }
        };

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
        // drop the prepared statement before releasing the connection lock
        drop(stmt);
        drop(conn_guard);

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

    /// Query records for a specific checkpoint start (UTC+8 04:00 of the provided date).
    /// If `date_str` is None, uses get_checkpoint() (today by bot rules).
    /// Returns a Vec of (nickname, Option<HH:MM string>) where None means no record.
    pub fn query_records_for_date(
        &self,
        date_str: Option<&str>,
    ) -> Result<Vec<(String, Option<String>)>, String> {
        let checkpoint_start = match date_str {
            Some(s) => match chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                Ok(d) => BOT_TZ
                    .from_local_datetime(&NaiveDateTime::new(d, BOT_CHECKPOINT))
                    .single()
                    .ok_or_else(|| "invalid date/time".to_string())?,
                Err(e) => return Err(format!("invalid date: {:?}", e)),
            },
            None => get_checkpoint(),
        };

        let checkpoint_end = checkpoint_start + chrono::Duration::days(1);

        let conn_guard = self.conn.lock().unwrap();
        // Order by presence of created_at (not null first) then by created_at asc, then by sort_key and id
        let mut stmt = conn_guard.prepare_cached(
            "SELECT `bot_group_member`.`group_nickname`, D.`created_at` FROM `bot_group_member`
            LEFT JOIN (
                SELECT `created_at`, `user_id` FROM `bot_daka` WHERE `bot_daka`.`created_at` >= ?1 AND `bot_daka`.`created_at` < ?2
            ) D ON D.`user_id` = `bot_group_member`.`id`
            ORDER BY (D.`created_at` IS NULL), D.`created_at` ASC, `bot_group_member`.`sort_key` ASC, `bot_group_member`.`id` ASC",
        )
        .map_err(|e| format!("prepare failed: {:?}", e))?;

        let rows = stmt
            .query_map(
                params![checkpoint_start.naive_utc(), checkpoint_end.naive_utc()],
                |row| {
                    let nickname: String = row.get(0)?;
                    // read as UTC datetime and convert to BOT_TZ when present
                    let created_at: Option<chrono::DateTime<chrono::Utc>> = row.get(1)?;
                    let time_str =
                        created_at.map(|dt| dt.with_timezone(&BOT_TZ).format("%H:%M").to_string());
                    Ok((nickname, time_str))
                },
            )
            .map_err(|e| format!("query failed: {:?}", e))?;

        let mut out: Vec<(String, Option<String>)> = Vec::new();
        for r in rows {
            match r {
                Ok((n, t)) => out.push((n, t)),
                Err(e) => return Err(format!("row error: {:?}", e)),
            }
        }
        drop(stmt);
        drop(conn_guard);
        Ok(out)
    }

    /// Ensure the member record exists and update nickname/group_nickname.
    /// Returns the `id` of the bot_group_member on success or a ServiceResponse error.
    pub fn upsert_member(&self, group_member: &GroupMember) -> Result<i64, ServiceResponse> {
        let uid = &group_member.uid;
        let nickname = group_member.member_name.as_deref().unwrap_or_default();
        let group_nickname = group_member.member_card.as_deref().unwrap_or(nickname);

        const UPSERT_RECORD_SQL: &str =
            "INSERT INTO `bot_group_member` (`qq_uid`, `qq_uin`, `nickname`, `group_nickname`)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT (`qq_uid`)
                DO UPDATE SET `nickname` = excluded.nickname, `group_nickname` = excluded.group_nickname
            RETURNING `id`";

        let conn_guard = self.conn.lock().unwrap();
        let mut stmt = match conn_guard.prepare_cached(UPSERT_RECORD_SQL) {
            Ok(s) => s,
            Err(e) => {
                return Err(ServiceResponse::err(format!(
                    "Prepare statement failed: {:?}",
                    e
                )));
            }
        };
        let id: i64 = match stmt.query_row(
            params![uid, group_member.uin, nickname, group_nickname],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(e) => {
                return Err(ServiceResponse::err(format!(
                    "Failed to update member name: {:?}",
                    e
                )));
            }
        };
        drop(stmt);
        drop(conn_guard);
        Ok(id)
    }

    pub fn handle_我没打卡(&self, user_id: i64, _args: &str) -> ServiceResponse {
        let checkpoint = get_checkpoint();

        let conn_guard = self.conn.lock().unwrap();

        let mut 我没打卡_stmt = conn_guard
            .prepare_cached("DELETE FROM `bot_daka` WHERE `user_id` = ?1 AND `created_at` >= ?2")
            .expect("Prepare statement failed");

        let res = 我没打卡_stmt.execute(params![user_id, checkpoint.naive_utc()]);
        drop(我没打卡_stmt);
        drop(conn_guard);
        let msg = match res {
            Ok(0) => ServiceResponse::ok("确实"),
            Ok(_) => ServiceResponse::ok("行吧"),
            Err(e) => {
                tracing::error!("Failed to insert record: {:?}", e);
                ServiceResponse::err("我没打卡失败：数据库错误")
            }
        };

        msg
    }

    pub fn handle_打卡(&self, user_id: i64, _args: &str) -> ServiceResponse {
        let checkpoint = get_checkpoint();

        let conn_guard = self.conn.lock().unwrap();

        let mut 打卡_stmt = conn_guard
            .prepare_cached(
                "INSERT INTO `bot_daka` (`user_id`) SELECT ?1 WHERE NOT EXISTS (
            SELECT 1 FROM `bot_daka` WHERE `user_id` = ?1 AND `created_at` >= ?2
        )",
            )
            .expect("Prepare statement failed");

        let res = 打卡_stmt.execute(params![user_id, checkpoint.naive_utc()]);
        drop(打卡_stmt);
        drop(conn_guard);

        let mut _daily_report = String::new();
        let msg = match res {
            Ok(0) => ServiceResponse::ok("您今天已经打过卡莉"),
            Ok(_) => {
                _daily_report = self.build_daily_report();
                ServiceResponse::ok(_daily_report.clone())
            }
            Err(e) => {
                tracing::error!("Failed to insert record: {:?}", e);
                ServiceResponse::err("打卡失败：数据库错误")
            }
        };

        msg
    }

    pub fn handle_咕(
        &self,
        _group_uin: u32,
        _group_member: &GroupMember,
        _args: &str,
    ) -> ServiceResponse {
        // reuse the query logic to obtain lists and format the message
        match self.query_missed_and_warning() {
            Ok((missed, warn)) => {
                if missed.is_empty() && warn.is_empty() {
                    ServiceResponse::ok("没有人咕咕".to_string())
                } else {
                    let failed_msg = if missed.is_empty() {
                        "".to_string()
                    } else {
                        format!("💢 10天没打卡：\n{}", missed.join("\u{3000}"))
                    };
                    let warning_msg = if warn.is_empty() {
                        "".to_string()
                    } else {
                        format!("⚠️ 7天没打卡：\n{}", warn.join("\u{3000}"))
                    };
                    ServiceResponse::ok(format!("{}\n{}", failed_msg, warning_msg))
                }
            }
            Err(e) => {
                error!("Failed to query records for 咕: {:?}", e);
                ServiceResponse::err("咕咕查询失败：数据库错误")
            }
        }
    }

    /// Return two lists: missed in last 10 days (never daka in window) and warning list (last daka older than 7 days)
    pub fn query_missed_and_warning(&self) -> Result<(Vec<String>, Vec<String>), String> {
        let checkpoint_end = get_checkpoint();
        let checkpoint_start = checkpoint_end - chrono::Duration::days(10);

        let conn_guard = self.conn.lock().unwrap();
        let mut get_records_10day_stmt = conn_guard
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
            .map_err(|e| format!("prepare failed: {:?}", e))?;

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
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            .map_err(|e| format!("query failed: {:?}", e))?;
        drop(get_records_10day_stmt);
        drop(conn_guard);

        let failed_group_members = res
            .iter()
            .filter(|record| {
                if let Some(last_daka_at) = record.last_daka_at {
                    last_daka_at < checkpoint_start
                } else {
                    true
                }
            })
            .map(|record| record.group_nickname.clone())
            .collect::<Vec<_>>();

        let warning_checkpoint = checkpoint_end - chrono::Duration::days(7);
        let warning_group_members = res
            .iter()
            .filter_map(|record| {
                (record.last_daka_at? < warning_checkpoint).then_some(record.group_nickname.clone())
            })
            .collect::<Vec<_>>();

        Ok((failed_group_members, warning_group_members))
    }
}
