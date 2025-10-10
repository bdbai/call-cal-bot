pub mod daka;
pub mod models;
pub mod user;

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

#[derive(Clone)]
pub struct Service {
    conn: Arc<Mutex<Connection>>,
}

impl Service {
    pub(super) fn new(conn: Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    // Get password hash by member id
    pub fn get_password_by_id(&self, member_id: i64) -> Option<String> {
        let conn_guard = self.conn.lock().unwrap();
        let mut stmt = match conn_guard
            .prepare_cached("SELECT password FROM bot_group_member WHERE id = ?1")
        {
            Ok(s) => s,
            Err(_) => return None,
        };
        let res: Result<String, _> = stmt.query_row([member_id], |r| r.get(0));
        drop(stmt);
        drop(conn_guard);
        res.ok()
    }

    // Update password by id
    pub fn update_password_by_id(&self, member_id: i64, hashed: &str) -> Result<(), String> {
        let conn_guard = self.conn.lock().unwrap();
        let mut stmt = conn_guard
            .prepare_cached("UPDATE bot_group_member SET password = ?1 WHERE id = ?2")
            .map_err(|e| format!("prepare failed: {:?}", e))?;
        let res = stmt
            .execute(rusqlite::params![hashed, member_id])
            .map_err(|e| format!("execute failed: {:?}", e))?;
        drop(stmt);
        drop(conn_guard);
        if res == 0 {
            Err("no rows updated".to_string())
        } else {
            Ok(())
        }
    }
}

pub fn init_service() -> Service {
    let mut conn = Connection::open("call-cal-bot.db").expect("Failed to open database");
    crate::migrations::runner()
        .run(&mut conn)
        .expect("db migration");
    Service::new(conn)
}
